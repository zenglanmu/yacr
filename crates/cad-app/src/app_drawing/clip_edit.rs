//! `CLIP` and `CLIPIT`: the generic boundary commands over the **existing** clip
//! models.
//!
//! This build already stores and renders two clips, and those two are the only
//! clip models it has:
//!
//! * [`cad_domain::ImageClip`], written by `IMAGECLIP`
//!   ([`super::image_edit`]) on one selected raster image;
//! * [`cad_domain::BlockClip`], written by `XCLIP` ([`super::xclip_edit`]) as a
//!   display attribute of one selected block reference.
//!
//! `CLIP`/`CLIPIT` therefore never define a second model: they resolve the
//! target from the current selection, build the boundary that target's model can
//! hold, and hand it to [`Application::write_image_clip`] /
//! [`Application::write_block_clip`] — the same validation, transaction and undo
//! patch the two specific commands use.
//!
//! # What each model can actually draw
//!
//! * An **image** clip is a pixel-space ring. The representation layer
//!   (`cad_representation::provider::image::image_clip_vertices`) fan-triangulates
//!   it, so only a **convex** `inside` ring is drawn. Both commands therefore
//!   check the convexity of the **pixel-space** ring [`super::image_edit::polygon_clip`]
//!   produced and refuse a non-convex one before the transaction
//!   (`clip.polygon_non_convex` / `clipit.polygon_non_convex`) instead of storing
//!   a boundary that would only be reported later. `IMAGECLIP` itself keeps
//!   storing and reporting (`image.clip_unsupported`, frame only) — unchanged.
//! * A **block reference** clip is drawn only through
//!   `cad_representation::block_clip_rect`, which accepts the two-corner
//!   rectangle form when the transformed corners stay an axis-aligned world-XY
//!   rectangle and **refuses a polygon** (`BlockClipRejection::Polygon`). A stored
//!   block polygon therefore never cuts anything, so this module refuses to store
//!   one: "accepted" and "drawn" stay the same statement.
//!
//! # `CLIP` (typed boundary)
//!
//! * `CLIP RECT <a> <b> <c> <d>` — the target's own clip space: **pixel** space
//!   for an image, **block-local** space for a block reference (exactly what
//!   `IMAGECLIP` / `XCLIP RECT` read). A block rectangle that does not map to an
//!   axis-aligned world-XY rectangle is refused with `XCLIP`'s own codes
//!   (`xclip.rotation_not_renderable` / `.non_xy_plane` / `.degenerate_transform`)
//!   before the transaction.
//! * `CLIP POLY <x1> <y1> <x2> <y2> <x3> <y3> …` — **world-space** points (what
//!   `IMAGECLIP POLY` reads), on **raster images only**: that is the model whose
//!   representation draws a polygon. On a block reference the word is refused by
//!   name (`clip.polygon_block_not_renderable`) and nothing is written; use
//!   `CLIP RECT` there. The mapped pixel-space ring must be convex
//!   (`clip.polygon_non_convex` otherwise) and simple (`imageclip.polygon_*`
//!   refusals), because that is what the renderer can fill.
//! * `CLIP ON|OFF` — only a block reference can express it (`BlockClip::visible`).
//!   An image clip has no enabled/disabled flag in this build, so the word is
//!   refused by name (`clip.on_unsupported` / `clip.off_unsupported`) instead of
//!   being reinterpreted as a delete;
//! * `CLIP DELETE` — removes the stored clip (both models can express that).
//!
//! # `CLIPIT` (boundary from the selection)
//!
//! The same selection must contain exactly one clippable target **plus** the
//! boundary entities: `LINE`, `ARC` and polyline edges. The edges are stitched by
//! the shared JOIN chainer ([`super::join::chain_pieces`] over
//! [`super::join::piece_from_geometry`]), the ring must close
//! (`clipit.open_boundary` otherwise) and be a simple, non-degenerate polygon
//! ([`crate::app_layout_geometry::validate_clip_polygon`], the same rule `VPCLIP`
//! uses). Arc/bulged segments are chord-approximated at the declared
//! [`CLIP_ARC_CHORDS`] resolution and the command says so in its
//! `clipit.boundary` diagnostic.
//!
//! `CLIPIT` always builds a **polygon**, so its target must be a raster image. A
//! block reference target is refused by name
//! (`clipit.polygon_block_not_renderable`) before the boundary is even built — a
//! block polygon would be stored data with no render path. The built ring must
//! also be convex in the image's pixel space (`clipit.polygon_non_convex`), the
//! same requirement `CLIP POLY` applies.
//!
//! # Stable refusal codes
//!
//! `clip.payload` / `.no_selection` / `.multiple` / `.not_clippable` /
//! `.on_unsupported` / `.off_unsupported` / `.polygon_block_not_renderable` /
//! `.polygon_non_convex`, and
//! `clipit.payload` / `.not_clippable` / `.multiple_targets` /
//! `.unsupported_boundary` / `.no_boundary` / `.open_boundary` /
//! `.boundary_closed` / `.degenerate_boundary` / `.degenerate_polygon` /
//! `.non_coplanar` / `.self_intersecting` /
//! `.polygon_block_not_renderable` / `.polygon_non_convex`. The target-specific
//! boundary and write refusals keep their existing `imageclip.*` / `xclip.*`
//! codes, because those are the models being written.
//!
//! Every refusal happens before a transaction, so a refused command writes
//! nothing, bumps no revision and records no history. Both commands are
//! **Work-only** (Viewer mode is refused before the handler runs) and touch no
//! session state beyond the document they write.

use super::join::{arc_point, chain_pieces, piece_from_geometry};
use super::*;
use crate::app_utility::require_none_payload;
use crate::command_payload::{ClipCommand, ImageClipCommand, XClipCommand};
use cad_domain::SpaceId;
use cad_geometry::bulge_arc_geometry;

/// How many chords one arc/bulged boundary segment is approximated with.
///
/// 32 chords per arc is the same angular resolution the HATCH ellipse sampling
/// uses (64 chords for a full turn, i.e. 5.6 degrees per chord), so a curved clip
/// boundary and a sampled hatch edge agree about what "curved" means here. The
/// approximation is declared in the `clipit.boundary` diagnostic, never hidden.
const CLIP_ARC_CHORDS: usize = 32;

/// The clip commands (`CLIP`, `CLIPIT`); both are Work-only.
impl Application {
    /// Dispatch one clipping command.
    pub(crate) fn clip_edit(
        &mut self,
        session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::Clip => self.clip(session, command),
            ExtendedCommandId::ClipIt => self.clipit(session, command),
            other => Err(CadError::Invariant(format!(
                "not a clipping command: {other:?}"
            ))),
        }
    }

    /// `CLIP RECT|POLY|ON|OFF|DELETE` on the single selected clippable object.
    ///
    /// `RECT` and the three state words work on both models. `POLY` is the raster
    /// image form: a block reference has no renderable polygon clip, so it is
    /// refused by name (`clip.polygon_block_not_renderable`) before the write, and
    /// the image ring itself must be convex in pixel space
    /// (`clip.polygon_non_convex`) because the renderer fan-triangulates it.
    fn clip(&mut self, session: &SessionState, command: &Command) -> CadResult<CommandOutcome> {
        let CommandPayload::Clip(payload) = &command.payload else {
            return Err(clip_refusal(
                "clip.payload",
                "CLIP needs a Clip payload (`RECT <minx> <miny> <maxx> <maxy>`, `POLY <x1> \
                 <y1> <x2> <y2> <x3> <y3> …`, `ON`, `OFF` or `DELETE`); the current selection is \
                 the one image or block reference to clip",
            ));
        };
        let request = payload.as_ref();
        let (document_id, candidate, before) =
            self.clip_target(session, command, "CLIP", "clip")?;
        let entity = before.id.0;
        let object = Some(ObjectId(entity));
        if matches!(before.geometry, SemanticGeometry::Insert { .. })
            && before.space != SpaceId::Model
        {
            return Err(clip_refusal(
                "clip.not_clippable",
                format!(
                    "entity {entity} is a block member or paper-space reference, not a model-space \
                     block reference; nothing was written"
                ),
            ));
        }
        match &before.geometry {
            SemanticGeometry::Image { origin, u, v, .. } => {
                let image_request = match request {
                    ClipCommand::Rect(rect) => ImageClipCommand::Rect(*rect),
                    ClipCommand::Polygon(points) => {
                        // The representation fan-triangulates an image clip ring, so
                        // only a **convex** ring is drawn. Map the world ring through
                        // the very same basis inverse the write uses (one mapping, not
                        // two) and apply that requirement to the pixel-space ring —
                        // refusing here instead of storing a boundary that would only
                        // be reported later as `image.clip_unsupported`.
                        let clip = super::image_edit::polygon_clip(points, *origin, *u, *v)?;
                        if !pixel_ring_is_convex(&clip.vertices) {
                            return Err(clip_refusal(
                                "clip.polygon_non_convex",
                                format!(
                                    "CLIP POLY: image entity {entity} would store a non-convex \
                                     clip ring ({} pixel-space vertices, at least one reflex \
                                     turn); the representation fan-triangulates an image clip, so \
                                     only a convex ring is drawn and nothing was written; draw the \
                                     boundary as a convex polygon or use `CLIP RECT`",
                                    clip.vertices.len()
                                ),
                            ));
                        }
                        ImageClipCommand::Polygon(points.clone())
                    }
                    ClipCommand::Delete => ImageClipCommand::Off,
                    ClipCommand::On => {
                        return Err(clip_refusal(
                            "clip.on_unsupported",
                            format!(
                                "CLIP ON: image entity {entity} has no enabled/disabled clip flag \
                                 in this build (the pixel-space clip is either stored or absent), \
                                 so nothing was written; use `CLIP POLY …`/`CLIP RECT …` to set a \
                                 clip or `CLIP DELETE` to clear it"
                            ),
                        ));
                    }
                    ClipCommand::Off => {
                        return Err(clip_refusal(
                            "clip.off_unsupported",
                            format!(
                                "CLIP OFF: image entity {entity} has no enabled/disabled clip flag \
                                 in this build (the pixel-space clip is either stored or absent), \
                                 so nothing was written; `CLIP DELETE` clears it instead"
                            ),
                        ));
                    }
                };
                let mut outcome =
                    self.write_image_clip(document_id, candidate, before, &image_request)?;
                outcome.diagnostics.insert(
                    0,
                    Diagnostic {
                        object,
                        code: "clip.applied".into(),
                        message: format!(
                            "CLIP: the selection is image entity {entity}; the pixel-space \
                             boundary is stored through the IMAGECLIP model"
                        ),
                    },
                );
                Ok(outcome)
            }
            SemanticGeometry::Insert { .. } => {
                let block_request = match request {
                    ClipCommand::Rect(rect) => XClipCommand::Rect(*rect),
                    // A block polygon boundary is stored data the representation
                    // layer cannot render (`block_clip_rect` refuses
                    // `BlockClipRejection::Polygon`, so the reference would draw
                    // unclipped). Refusing it here — before the transaction —
                    // keeps "accepted" and "drawn" the same statement.
                    ClipCommand::Polygon(_) => {
                        return Err(clip_refusal(
                            "clip.polygon_block_not_renderable",
                            format!(
                                "CLIP POLY: block reference {entity} can only be clipped by the \
                                 rectangular boundary this build renders; a block-local polygon is \
                                 stored data with no render path (the representation layer refuses \
                                 it and the reference would draw unclipped), so nothing was \
                                 written; use `CLIP RECT <minx> <miny> <maxx> <maxy>` in \
                                 block-local coordinates"
                            ),
                        ));
                    }
                    ClipCommand::On => XClipCommand::On,
                    ClipCommand::Off => XClipCommand::Off,
                    ClipCommand::Delete => XClipCommand::Delete,
                };
                Self::validate_xclip_boundary(&block_request)?;
                let mut outcome =
                    self.write_block_clip(document_id, candidate, before, &block_request)?;
                outcome.diagnostics.insert(
                    0,
                    Diagnostic {
                        object,
                        code: "clip.applied".into(),
                        message: format!(
                            "CLIP: the selection is block reference {entity}; the block-local \
                             boundary is stored through the XCLIP model"
                        ),
                    },
                );
                Ok(outcome)
            }
            other => Err(clip_refusal(
                "clip.not_clippable",
                not_clippable_detail(entity, class_name(other), before.object.type_key.as_str()),
            )),
        }
    }

    /// `CLIPIT` on the single selected target plus the selected boundary entities.
    fn clipit(&mut self, session: &SessionState, command: &Command) -> CadResult<CommandOutcome> {
        require_none_payload(command, "CLIPIT")?;
        let (document_id, candidate, target, boundary) = self.clipit_selection(session, command)?;
        // The built boundary is always a polygon, and a block reference has no
        // renderable polygon clip: `block_clip_rect` refuses
        // `BlockClipRejection::Polygon`, so the stored boundary would never be
        // drawn. Refuse the kind before the boundary is even built — no
        // transaction, no stored clip, and no "accepted but unclipped" reference.
        if matches!(target.geometry, SemanticGeometry::Insert { .. }) {
            return Err(clip_refusal(
                "clipit.polygon_block_not_renderable",
                format!(
                    "CLIPIT: the clip target must be a raster image; block reference {} can only \
                     be clipped by the rectangular boundary this build renders, and CLIPIT always \
                     builds a polygon, so nothing was written; use `CLIP RECT <minx> <miny> \
                     <maxx> <maxy>` on the reference (block-local coordinates) instead",
                    target.id.0
                ),
            ));
        }
        let ring = boundary_ring(&boundary)?;
        let object = Some(ObjectId(target.id.0));
        let mut outcome = match &target.geometry {
            SemanticGeometry::Image { origin, u, v, .. } => {
                // `CLIPIT` always stores a polygon, so the image ring must satisfy
                // the representation's convexity requirement before the write (the
                // boundary is simple by `validate_clip_polygon`, and this maps it
                // through the one pixel-space basis inverse).
                let clip = super::image_edit::polygon_clip(&ring, *origin, *u, *v)?;
                if !pixel_ring_is_convex(&clip.vertices) {
                    return Err(clip_refusal(
                        "clipit.polygon_non_convex",
                        format!(
                            "CLIPIT: the selected boundary is non-convex in image entity {}'s \
                             pixel space ({} vertices, at least one reflex turn); the \
                             representation fan-triangulates an image clip, so only a convex \
                             boundary is drawn and nothing was written",
                            target.id.0,
                            clip.vertices.len()
                        ),
                    ));
                }
                let request = ImageClipCommand::Polygon(ring.clone());
                self.write_image_clip(document_id, candidate, target.clone(), &request)?
            }
            other => {
                return Err(clip_refusal(
                    "clipit.not_clippable",
                    not_clippable_detail(
                        target.id.0,
                        class_name(other),
                        target.object.type_key.as_str(),
                    ),
                ));
            }
        };
        let approximation = if boundary_has_arcs(&boundary) {
            format!("; arc/bulged edges are {CLIP_ARC_CHORDS}-chord approximations")
        } else {
            String::new()
        };
        outcome.diagnostics.insert(
            0,
            Diagnostic {
                object,
                code: "clipit.boundary".into(),
                message: format!(
                    "CLIPIT: built a closed {}-vertex boundary from {} selected {} entit{} and \
                     clipped image {}{approximation}",
                    ring.len(),
                    boundary.len(),
                    boundary_kinds(&boundary),
                    if boundary.len() == 1 { "y" } else { "ies" },
                    target.id.0
                ),
            },
        );
        Ok(outcome)
    }

    /// The single directly addressable selected clippable entity, plus a mutable
    /// drawing candidate cloned from the document.
    ///
    /// Shared by both commands so the selection refusals
    /// (`<code>.no_selection` / `.multiple` / `.not_clippable`) are worded and
    /// coded once. Nothing is written here.
    fn clip_target(
        &self,
        session: &SessionState,
        command: &Command,
        name: &str,
        code: &str,
    ) -> CadResult<(DocumentId, cad_db::DrawingDatabase, DbEntity)> {
        let references = session.selection.refs();
        let reference = match references {
            [] => {
                return Err(clip_refusal(
                    format!("{code}.no_selection"),
                    format!(
                        "{name} needs exactly one selected image or block reference; nothing is \
                         selected"
                    ),
                ));
            }
            [one] => one,
            many => {
                return Err(clip_refusal(
                    format!("{code}.multiple"),
                    format!(
                        "{name} clips exactly one image or block reference; {} entities are \
                         selected",
                        many.len()
                    ),
                ));
            }
        };
        if reference.document != command.document
            || !reference.instance.0.is_empty()
            || reference.sub_element.is_some()
        {
            return Err(clip_refusal(
                format!("{code}.not_clippable"),
                format!(
                    "entity {} is not a directly addressable object in this document",
                    reference.entity.0
                ),
            ));
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let candidate = document.drawing.as_ref().clone();
        let before = candidate.entity(reference.entity).cloned().ok_or_else(|| {
            clip_refusal(
                format!("{code}.not_clippable"),
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
        Ok((document_id, candidate, before))
    }

    /// The `CLIPIT` selection split: exactly one clippable target plus at least
    /// one boundary entity, both from the current selection.
    fn clipit_selection(
        &self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<(DocumentId, cad_db::DrawingDatabase, DbEntity, Vec<DbEntity>)> {
        let references = session.selection.refs();
        if references.is_empty() {
            return Err(clip_refusal(
                "clipit.not_clippable",
                "CLIPIT needs one selected image or block reference plus the boundary entities; \
                 nothing is selected",
            ));
        }
        let document = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let candidate = document.drawing.as_ref().clone();
        let mut target: Option<DbEntity> = None;
        let mut boundary = Vec::new();
        let mut unsupported: Option<(u128, &'static str, String)> = None;
        for reference in references {
            if reference.document != command.document
                || !reference.instance.0.is_empty()
                || reference.sub_element.is_some()
            {
                return Err(clip_refusal(
                    "clipit.not_clippable",
                    format!(
                        "entity {} is not a directly addressable object in this document",
                        reference.entity.0
                    ),
                ));
            }
            let entity = candidate.entity(reference.entity).cloned().ok_or_else(|| {
                clip_refusal(
                    "clipit.not_clippable",
                    format!(
                        "entity {} does not exist in this drawing",
                        reference.entity.0
                    ),
                )
            })?;
            match &entity.geometry {
                SemanticGeometry::Image { .. } => {
                    if target.is_some() {
                        return Err(clip_refusal(
                            "clipit.multiple_targets",
                            format!(
                                "CLIPIT clips exactly one image or block reference; entity {} is a \
                                 second clippable target",
                                entity.id.0
                            ),
                        ));
                    }
                    target = Some(entity);
                }
                SemanticGeometry::Insert { .. } => {
                    if entity.space != SpaceId::Model {
                        return Err(clip_refusal(
                            "clipit.not_clippable",
                            format!(
                                "entity {} is a block member or paper-space reference, not a \
                                 model-space block reference",
                                entity.id.0
                            ),
                        ));
                    }
                    if target.is_some() {
                        return Err(clip_refusal(
                            "clipit.multiple_targets",
                            format!(
                                "CLIPIT clips exactly one image or block reference; entity {} is a \
                                 second clippable target",
                                entity.id.0
                            ),
                        ));
                    }
                    target = Some(entity);
                }
                SemanticGeometry::Line { .. }
                | SemanticGeometry::Arc { .. }
                | SemanticGeometry::Polyline { .. } => boundary.push(entity),
                other => {
                    unsupported = Some((
                        entity.id.0,
                        class_name(other),
                        entity.object.type_key.clone(),
                    ));
                }
            }
        }
        let Some(target) = target else {
            let detail = match unsupported {
                Some((id, class, key)) => not_clippable_detail(id, class, &key),
                None => "CLIPIT needs one selected image or block reference as the clip target, \
                         plus the boundary entities"
                    .to_string(),
            };
            return Err(clip_refusal("clipit.not_clippable", detail));
        };
        if let Some((id, class, key)) = unsupported {
            return Err(clip_refusal(
                "clipit.unsupported_boundary",
                format!(
                    "entity {id} is a {class} ({key}); CLIPIT builds its boundary from LINE, ARC \
                     and polyline entities only, so nothing was written"
                ),
            ));
        }
        if boundary.is_empty() {
            return Err(clip_refusal(
                "clipit.no_boundary",
                format!(
                    "CLIPIT needs at least one selected LINE, ARC or polyline boundary around the \
                     target; only the target (entity {}) is selected",
                    target.id.0
                ),
            ));
        }
        Ok((command.document, candidate, target, boundary))
    }
}

/// One stable, code-prefixed refusal for `CLIP` / `CLIPIT`.
fn clip_refusal(code: impl std::fmt::Display, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// The one sentence every "this object cannot be clipped" refusal shares.
fn not_clippable_detail(entity: u128, class: &str, type_key: &str) -> String {
    format!(
        "entity {entity} is a {class} ({type_key}); CLIP/CLIPIT clip a raster image or a \
         model-space block reference, and PDF/DGN/DWF underlay clipping has no model in this \
         build, so nothing was written"
    )
}

/// Whether a **pixel-space** ring is convex, in the exact sense the image clip
/// renderer needs.
///
/// This mirrors `cad_representation::provider::image::polygon_is_convex` — the
/// renderer fan-triangulates an image clip polygon, so only a convex ring is a
/// visible-region polygon it can fill. The rule is the same one:
///
/// * fewer than three vertices is trivially convex (the parser and
///   [`super::image_edit::polygon_clip`] refuse those first);
/// * consecutive **collinear** edges are allowed (a zero cross product does not
///   fix a winding sign, tolerance `1e-9 * |e1| * |e2|`);
/// * any sign change between consecutive edge cross products is a reflex vertex,
///   i.e. non-convex.
///
/// It is deliberately a plain test, not a second triangulator: the points it reads
/// are the pixel-space vertices [`super::image_edit::polygon_clip`] produced, so
/// the check and the write cannot disagree about the ring. Self-intersection is
/// *not* re-checked here: `polygon_clip` already refuses it
/// (`imageclip.polygon_self_intersection`) on the same pixel ring, before the
/// write.
fn pixel_ring_is_convex(ring: &[[f64; 2]]) -> bool {
    let count = ring.len();
    if count < 3 {
        return true;
    }
    let mut sign = 0i8;
    for index in 0..count {
        let a = ring[index];
        let b = ring[(index + 1) % count];
        let c = ring[(index + 2) % count];
        let e1 = (b[0] - a[0], b[1] - a[1]);
        let e2 = (c[0] - b[0], c[1] - b[1]);
        let length = e1.0.hypot(e1.1) * e2.0.hypot(e2.1);
        if length <= 0.0 {
            // A repeated vertex has no turn to classify.
            continue;
        }
        let cross = e1.0 * e2.1 - e1.1 * e2.0;
        if cross.abs() <= 1e-9 * length {
            // Collinear: allowed, does not fix the winding sign.
            continue;
        }
        let direction = if cross > 0.0 { 1 } else { -1 };
        if sign == 0 {
            sign = direction;
        } else if sign != direction {
            return false;
        }
    }
    true
}

/// Whether any selected boundary entity carries a curved segment (an `ARC` or a
/// non-zero polyline bulge), i.e. whether the ring was chord-approximated.
fn boundary_has_arcs(boundary: &[DbEntity]) -> bool {
    boundary.iter().any(|entity| match &entity.geometry {
        SemanticGeometry::Arc { .. } => true,
        SemanticGeometry::Polyline { bulges, .. } => {
            bulges.iter().any(|bulge| bulge.abs() >= 1e-12)
        }
        _ => false,
    })
}

/// The distinct class names of the boundary entities, for the diagnostic.
fn boundary_kinds(boundary: &[DbEntity]) -> String {
    let mut kinds: Vec<&'static str> = Vec::new();
    for entity in boundary {
        let kind = class_name(&entity.geometry);
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    kinds.join("/")
}

/// Build the closed boundary ring of a `CLIPIT` selection.
///
/// A single closed polyline is its own ring; every other combination is stitched
/// by the shared JOIN chainer, so the tolerance and the touching rule are the
/// ones `JOIN`/`PEDIT JOIN` already use. The result must close
/// (`clipit.open_boundary`), be a simple non-degenerate ring
/// ([`crate::app_layout_geometry::validate_clip_polygon`]) and is returned with
/// the repeated closing vertex dropped (the open form every clip model stores).
fn boundary_ring(boundary: &[DbEntity]) -> CadResult<Vec<Point3>> {
    let (points, bulges) = match boundary {
        [one] => single_boundary_ring(one)?,
        many => chained_boundary_ring(many)?,
    };
    let mut ring = flatten_ring(&points, &bulges)?;
    // A closed polyline may store the start vertex twice; every clip model keeps
    // the open ring, so drop that trailing duplicate before validating.
    if ring.len() >= 2 && distance(ring[0], ring[ring.len() - 1]) <= super::join::join_tolerance() {
        ring.pop();
    }
    crate::app_layout_geometry::validate_clip_polygon(&ring, "clipit")?;
    Ok(ring)
}

/// The `(points, bulges)` of a one-entity boundary.
fn single_boundary_ring(entity: &DbEntity) -> CadResult<(Vec<Point3>, Vec<f64>)> {
    match &entity.geometry {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
            ..
        } => {
            if !closed {
                return Err(clip_refusal(
                    "clipit.open_boundary",
                    format!(
                        "CLIPIT boundary entity {} is an open polyline; a clip boundary must be \
                         closed",
                        entity.id.0
                    ),
                ));
            }
            let mut bulges = bulges.clone();
            if bulges.len() < points.len() {
                bulges.resize(points.len(), 0.0);
            }
            Ok((points.clone(), bulges))
        }
        other => Err(clip_refusal(
            "clipit.open_boundary",
            format!(
                "the only selected boundary entity {} is a {}; a single LINE or ARC cannot close a \
                 clip boundary",
                entity.id.0,
                class_name(other)
            ),
        )),
    }
}

/// The `(points, bulges)` of a multi-entity boundary, stitched by the JOIN
/// chainer.
fn chained_boundary_ring(boundary: &[DbEntity]) -> CadResult<(Vec<Point3>, Vec<f64>)> {
    let mut pieces = Vec::with_capacity(boundary.len());
    for entity in boundary {
        if matches!(
            entity.geometry,
            SemanticGeometry::Polyline { closed: true, .. }
        ) {
            return Err(clip_refusal(
                "clipit.boundary_closed",
                format!(
                    "boundary entity {} is a closed polyline and cannot be chained with the \
                     other selected edges; select either that one closed polyline or the open \
                     edges, not both",
                    entity.id.0
                ),
            ));
        }
        pieces.push(piece_from_geometry(&entity.geometry, "clipit")?);
    }
    let mut chain = chain_pieces(pieces, "clipit")?;
    let tolerance = super::join::join_tolerance();
    let first = chain.points[0];
    let last = *chain.points.last().expect("a chained piece has points");
    if distance(first, last) > tolerance {
        return Err(clip_refusal(
            "clipit.open_boundary",
            format!(
                "the selected boundary entities do not close: the chain ends at ({}, {}, {}) but \
                 starts at ({}, {}, {}), {} world units apart",
                last.x,
                last.y,
                last.z,
                first.x,
                first.y,
                first.z,
                distance(first, last)
            ),
        ));
    }
    // The chain repeats the start vertex at the end; a clip ring is stored open.
    // The last remaining bulge belongs to the closing edge (`points[n-1]` back to
    // `points[0]`), so only the duplicated vertex is dropped here.
    chain.points.pop();
    Ok((chain.points, chain.bulges))
}

/// Replace every arc/bulged segment with [`CLIP_ARC_CHORDS`] chords.
///
/// A clip ring is a straight-segment polygon (that is what both stored clip
/// models are), so a selected ARC can only enter it as a declared chord
/// approximation. The chords use the same plane frame the tessellator draws the
/// arc in, so the approximation is the curve the drawing itself shows.
fn flatten_ring(points: &[Point3], bulges: &[f64]) -> CadResult<Vec<Point3>> {
    if points.is_empty() {
        return Err(clip_refusal(
            "clipit.degenerate_boundary",
            "the selected boundary entities produced no vertices",
        ));
    }
    if bulges.iter().all(|bulge| bulge.abs() < 1e-12) {
        return Ok(points.to_vec());
    }
    let normal = cad_geometry::polyline_plane_normal(points);
    let count = points.len();
    let mut ring: Vec<Point3> = Vec::with_capacity(count + CLIP_ARC_CHORDS);
    for (index, point) in points.iter().enumerate() {
        ring.push(*point);
        let bulge = bulges.get(index).copied().unwrap_or(0.0);
        if bulge.abs() < 1e-12 {
            continue;
        }
        let next = points[(index + 1) % count];
        let Some(SemanticGeometry::Arc {
            center,
            normal: arc_normal,
            radius,
            start,
            sweep,
        }) = bulge_arc_geometry(*point, next, bulge, normal)
        else {
            return Err(clip_refusal(
                "clipit.degenerate_boundary",
                format!(
                    "the bulged boundary segment ({}, {}, {}) -> ({}, {}, {}) has no circular arc \
                     in the boundary plane",
                    point.x, point.y, point.z, next.x, next.y, next.z
                ),
            ));
        };
        for step in 1..CLIP_ARC_CHORDS {
            let angle = start + sweep * (step as f64) / (CLIP_ARC_CHORDS as f64);
            ring.push(arc_point(center, arc_normal, radius, angle));
        }
    }
    Ok(ring)
}
