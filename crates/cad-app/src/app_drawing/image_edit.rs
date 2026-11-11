//! `IMAGEATTACH`, `IMAGECLIP` and `IMAGEADJUST`: real raster-image authoring on
//! the existing `SemanticGeometry::Image` pipeline.
//!
//! All three commands are **Work-only** and write through the normal drawing
//! transaction path: an attach inserts one entity through the shared
//! [`Application::commit_new_entity`] funnel (active layer, model space, one
//! transaction / one undo step), and a clip or adjustment updates one selected
//! image row in one transaction / one undo step. Every refusal happens before a
//! transaction, so a refused command writes nothing, bumps no revision and
//! records no history.
//!
//! `IMAGE` / `-IMAGE` are the read-only report and live in
//! `crate::app_extended_w16_reports`; the quality/editor names remain named
//! refusals in `crate::app_extended_w16_refusals`.
//!
//! Stable refusal codes:
//!
//! * `imageattach.payload` / `.unsafe_reference` / `.invalid_origin` /
//!   `.invalid_pixels` / `.invalid_units_per_pixel` / `.invalid_extent`;
//! * `imageclip.payload` / `.no_selection` / `.multiple` / `.not_image` /
//!   `.invalid_rect` / `.polygon_points` / `.polygon_non_coplanar` /
//!   `.polygon_self_intersection` / `.degenerate_plane`;
//! * `imageadjust.payload` / `.no_selection` / `.multiple` / `.not_image` /
//!   `.invalid_value`.
//!
//! Texture resolution stays host-side: the stored entity carries only the
//! normalised logical key. When that key is not under the drawing directory the
//! representation layer emits the honest frame plus the existing
//! `image.unresolved` diagnostic — this command never fabricates a texture.

use super::helpers::offset_tolerance;
use super::*;
use crate::command_payload::{ImageAdjustCommand, ImageAttachCommand, ImageClipCommand};
use cad_domain::{DocumentId, ImageAdjust};

/// The image authoring/editing commands (Work-only).
impl Application {
    /// Dispatch one raster-image edit command.
    pub(crate) fn image_edit(
        &mut self,
        session: &SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::ImageAttach => self.image_attach(session, command),
            ExtendedCommandId::ImageClip => self.image_clip(session, command),
            ExtendedCommandId::ImageAdjust | ExtendedCommandId::DashImageAdjust => {
                self.image_adjust(session, command)
            }
            other => Err(CadError::Invariant(format!(
                "not a raster-image edit command: {other:?}"
            ))),
        }
    }

    /// `IMAGEATTACH <file> <x> <y> <width_px> <height_px> <units_per_pixel>`.
    ///
    /// Builds one `SemanticGeometry::Image` on the `+Z` work plane:
    /// `origin` is the typed point, `u`/`v` are the per-pixel edge vectors
    /// `(units_per_pixel, 0, 0)` and `(0, units_per_pixel, 0)`, `pixels` is the
    /// typed `(width, height)` in pixels, `file` is the **normalised** logical key
    /// and the clip starts empty with the image visible.
    ///
    /// The command is inserted through the shared create funnel, so it lands on
    /// the active layer in model space in one transaction / one undo step. All
    /// four named refusals are checked before that transaction.
    fn image_attach(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ImageAttach(payload) = &command.payload else {
            return Err(image_refusal(
                "imageattach.payload",
                "IMAGEATTACH needs an ImageAttach payload (file, x, y, width_px, \
                 height_px, units_per_pixel)",
            ));
        };
        let ImageAttachCommand {
            file,
            origin,
            pixels,
            units_per_pixel,
        } = payload.as_ref();
        let Some(key) = cad_resources::image_resource_key(file) else {
            return Err(image_refusal(
                "imageattach.unsafe_reference",
                format!(
                    "`{}` is not a safe relative resource key (absolute paths, `..` \
                     traversal and URLs are refused)",
                    file.trim()
                ),
            ));
        };
        if !is_finite_point(*origin) {
            return Err(image_refusal(
                "imageattach.invalid_origin",
                format!("the image origin {origin:?} is not finite"),
            ));
        }
        if !pixels[0].is_finite() || !pixels[1].is_finite() || pixels[0] <= 0.0 || pixels[1] <= 0.0
        {
            return Err(image_refusal(
                "imageattach.invalid_pixels",
                format!(
                    "the pixel dimensions must be positive and finite, got {} x {}",
                    pixels[0], pixels[1]
                ),
            ));
        }
        if !units_per_pixel.is_finite() || *units_per_pixel <= 0.0 {
            return Err(image_refusal(
                "imageattach.invalid_units_per_pixel",
                format!(
                    "units per pixel must be finite and strictly positive, got \
                     {units_per_pixel}"
                ),
            ));
        }
        // The world-space extent is `pixels * units_per_pixel`; both products
        // must stay finite or the frame/basis has no representable extent.
        let width = pixels[0] * units_per_pixel;
        let height = pixels[1] * units_per_pixel;
        if !width.is_finite() || !height.is_finite() {
            return Err(image_refusal(
                "imageattach.invalid_extent",
                format!(
                    "the world-space extent {} x {} (pixels x units_per_pixel) is not \
                     finite; reduce the pixel size or units per pixel",
                    pixels[0], pixels[1]
                ),
            ));
        }
        let stored_key = key.0;
        let geometry = SemanticGeometry::Image {
            origin: *origin,
            u: Point3 {
                x: *units_per_pixel,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: *units_per_pixel,
                z: 0.0,
            },
            pixels: *pixels,
            file: Some(stored_key.clone()),
            clip: None,
            visible: true,
            adjust: ImageAdjust::default(),
        };
        let mut outcome = self.commit_new_entity(session, command, geometry, "attach image")?;
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "imageattach.created".into(),
            message: format!(
                "IMAGEATTACH: attached `{stored_key}` as {} x {} px at ({}, {}, {}); \
                 extent {} x {} world units ({} per pixel); the texture is resolved from \
                 the drawing directory by the host, which reloads image resources after \
                 this command (unresolved keys appear in the host image report)",
                pixels[0], pixels[1], origin.x, origin.y, origin.z, width, height, units_per_pixel
            ),
        });
        Ok(outcome)
    }

    /// `IMAGECLIP <minx> <miny> <maxx> <maxy>` / `IMAGECLIP POLY …` / `IMAGECLIP OFF`.
    ///
    /// Pre-selection command: exactly one selected, directly addressable `Image`
    /// entity. Two boundary forms are stored, both with `inside: true`:
    ///
    /// * [`ImageClipCommand::Rect`] — the rectangle is in the image's **pixel
    ///   coordinate space** and is stored as its two opposite corners (the
    ///   renderer reads two vertices as a rectangle).
    /// * [`ImageClipCommand::Polygon`] — three or more **world-space** points.
    ///   Every point must lie on the image plane (the named
    ///   `imageclip.polygon_non_coplanar` refusal otherwise); each is mapped into
    ///   pixel space with the image's own `u`/`v` basis and stored as the N-gon.
    ///   Fewer than three distinct points is `imageclip.polygon_points`, an
    ///   edge-crossing boundary is `imageclip.polygon_self_intersection`, and a
    ///   degenerate image basis is `imageclip.degenerate_plane`.
    ///
    /// `OFF` clears the clip. An identical clip is `imageclip.unchanged` and opens
    /// no transaction.
    fn image_clip(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ImageClip(payload) = &command.payload else {
            return Err(image_refusal(
                "imageclip.payload",
                "IMAGECLIP needs an ImageClip payload (`<minx> <miny> <maxx> <maxy>`, `POLY <x1> \
                 <y1> <x2> <y2> <x3> <y3> …` or `OFF`)",
            ));
        };
        let request = payload.as_ref();
        let (document_id, mut candidate, before) =
            self.one_selected_image(session, command, "IMAGECLIP", "imageclip")?;
        let SemanticGeometry::Image {
            origin, u, v, clip, ..
        } = &before.geometry
        else {
            return Err(image_refusal(
                "imageclip.not_image",
                format!(
                    "entity {} is a {}, not a raster image",
                    before.id.0,
                    class_name(&before.geometry)
                ),
            ));
        };
        let new_clip = match request {
            ImageClipCommand::Off => None,
            ImageClipCommand::Rect([minx, miny, maxx, maxy]) => {
                if ![minx, miny, maxx, maxy]
                    .iter()
                    .all(|value| value.is_finite())
                    || minx >= maxx
                    || miny >= maxy
                {
                    return Err(image_refusal(
                        "imageclip.invalid_rect",
                        format!(
                            "the pixel-space rectangle must be finite with minx < maxx and \
                             miny < maxy, got [{minx}, {miny}] .. [{maxx}, {maxy}]"
                        ),
                    ));
                }
                Some(ImageClip {
                    vertices: vec![[*minx, *miny], [*maxx, *maxy]],
                    inside: true,
                })
            }
            ImageClipCommand::Polygon(points) => Some(polygon_clip(points, *origin, *u, *v)?),
        };
        let object = Some(ObjectId(before.id.0));
        if *clip == new_clip {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object,
                    code: "imageclip.unchanged".into(),
                    message: format!(
                        "IMAGECLIP: image entity {} already has this pixel-space clip; \
                         nothing was written",
                        before.id.0
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let mut after = before.clone();
        if let SemanticGeometry::Image { clip: slot, .. } = &mut after.geometry {
            *slot = new_clip.clone();
        }
        let label = if new_clip.is_some() {
            "image clip"
        } else {
            "image clip off"
        };
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
            staged.update_entity(after.clone())?;
            staged.commit()?
        };
        let stored = candidate.entity(after.id).cloned().ok_or_else(|| {
            CadError::Invariant("IMAGECLIP: the image vanished after commit".into())
        })?;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
                patches: vec![drawing_patch(before.id, Some(before.clone()), Some(stored))],
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        let (code, message) = match &new_clip {
            Some(clip) if clip.vertices.len() == 2 => (
                "imageclip.applied",
                format!(
                    "IMAGECLIP: set a pixel-space clip on image entity {} from ({}, {}) to \
                     ({}, {}); 2-vertex rectangle (inside)",
                    before.id.0,
                    clip.vertices[0][0],
                    clip.vertices[0][1],
                    clip.vertices[1][0],
                    clip.vertices[1][1]
                ),
            ),
            Some(clip) => (
                "imageclip.applied",
                format!(
                    "IMAGECLIP: set a pixel-space {}-vertex polygon clip on image entity {} \
                     (inside)",
                    clip.vertices.len(),
                    before.id.0
                ),
            ),
            None => (
                "imageclip.removed",
                format!(
                    "IMAGECLIP OFF: cleared the pixel-space clip on image entity {}",
                    before.id.0
                ),
            ),
        };
        Ok(CommandOutcome {
            objects: vec![ObjectId(before.id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object,
                code: code.into(),
                message,
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// `IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` /
    /// `IMAGEADJUST RESET`.
    ///
    /// Pre-selection command: exactly one selected, directly addressable `Image`
    /// entity. The three percentages are stored verbatim in the image's
    /// `adjust` field (`0..=100`, neutral `50`/`50`/`0`);
    /// `MONO`/`COLOR` are optional and an omitted word **keeps the image's
    /// current monochrome state** rather than resetting it. `RESET` restores the
    /// neutral 50/50/0/off transfer.
    ///
    /// A value above 100 is `imageadjust.invalid_value` (the payload is `u8`,
    /// whose type alone cannot express "at most 100"). An identical transfer is
    /// `imageadjust.unchanged` and opens no transaction, so a repeated command
    /// bumps neither the revision nor the undo stack.
    fn image_adjust(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ImageAdjust(payload) = &command.payload else {
            return Err(image_refusal(
                "imageadjust.payload",
                "IMAGEADJUST needs an ImageAdjust payload (`<brightness> <contrast> <fade> \
                 [MONO|COLOR]` or `RESET`)",
            ));
        };
        // The three percentages are validated **before** the selection is read:
        // a malformed transfer is the same refusal whether or not anything is
        // selected, and it never opens a transaction.
        if let ImageAdjustCommand::Set {
            brightness,
            contrast,
            fade,
            ..
        } = payload.as_ref()
        {
            if *brightness > 100 || *contrast > 100 || *fade > 100 {
                return Err(image_refusal(
                    "imageadjust.invalid_value",
                    format!(
                        "brightness, contrast and fade are percentages in 0..=100, got \
                         {brightness}/{contrast}/{fade}"
                    ),
                ));
            }
        }
        let (document_id, mut candidate, before) =
            self.one_selected_image(session, command, "IMAGEADJUST", "imageadjust")?;
        let current = match &before.geometry {
            SemanticGeometry::Image { adjust, .. } => *adjust,
            other => {
                return Err(image_refusal(
                    "imageadjust.not_image",
                    format!(
                        "entity {} is a {}, not a raster image",
                        before.id.0,
                        class_name(other)
                    ),
                ));
            }
        };
        let target = match payload.as_ref() {
            ImageAdjustCommand::Reset => ImageAdjust::default(),
            ImageAdjustCommand::Set {
                brightness,
                contrast,
                fade,
                monochrome,
            } => ImageAdjust {
                brightness: *brightness,
                contrast: *contrast,
                fade: *fade,
                // An omitted MONO/COLOR word keeps the stored state.
                monochrome: monochrome.unwrap_or(current.monochrome),
            },
        };
        let object = Some(ObjectId(before.id.0));
        let mode = |monochrome: bool| {
            if monochrome {
                "monochrome (MONO)"
            } else {
                "colour (COLOR)"
            }
        };
        if target == current {
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics: vec![Diagnostic {
                    object,
                    code: "imageadjust.unchanged".into(),
                    message: format!(
                        "IMAGEADJUST: image entity {} already has brightness {}, contrast {}, \
                         fade {}, {}; nothing was written",
                        before.id.0,
                        target.brightness,
                        target.contrast,
                        target.fade,
                        mode(target.monochrome)
                    ),
                }],
                measurement: None,
                point_report: None,
            });
        }
        let mut after = before.clone();
        if let SemanticGeometry::Image { adjust: slot, .. } = &mut after.geometry {
            *slot = target;
        }
        let label = "image adjust";
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = candidate.begin_drawing_transaction(label, transaction)?;
            staged.update_entity(after.clone())?;
            staged.commit()?
        };
        let stored = candidate.entity(after.id).cloned().ok_or_else(|| {
            CadError::Invariant("IMAGEADJUST: the image vanished after commit".into())
        })?;
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
                patches: vec![drawing_patch(before.id, Some(before.clone()), Some(stored))],
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;
        Ok(CommandOutcome {
            objects: vec![ObjectId(before.id.0)],
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object,
                code: "imageadjust.applied".into(),
                message: format!(
                    "IMAGEADJUST: image entity {} set to brightness {}, contrast {}, fade {}, \
                     {}; the renderer applies these in the declared order monochrome -> \
                     contrast -> brightness -> fade toward the background",
                    before.id.0,
                    target.brightness,
                    target.contrast,
                    target.fade,
                    mode(target.monochrome)
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }

    /// The one directly addressable selected `Image` entity, plus a mutable
    /// drawing candidate cloned from the document.
    ///
    /// Shared by every pre-selection image edit so the selection refusals
    /// (`<code>.no_selection` / `.multiple` / `.not_image`) are worded and coded
    /// once. Nothing is written here: the caller stages its own transaction.
    fn one_selected_image(
        &self,
        session: &SessionState,
        command: &Command,
        name: &str,
        code: &str,
    ) -> CadResult<(DocumentId, cad_db::DrawingDatabase, DbEntity)> {
        let references = session.selection.refs();
        let reference = match references {
            [] => {
                return Err(image_refusal(
                    format!("{code}.no_selection"),
                    format!("{name} needs exactly one selected image; nothing is selected"),
                ));
            }
            [one] => one,
            many => {
                return Err(image_refusal(
                    format!("{code}.multiple"),
                    format!(
                        "{name} edits exactly one image; {} entities are selected",
                        many.len()
                    ),
                ));
            }
        };
        if reference.document != command.document
            || !reference.instance.0.is_empty()
            || reference.sub_element.is_some()
        {
            return Err(image_refusal(
                format!("{code}.not_image"),
                format!(
                    "entity {} is not a directly addressable image in this document",
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
            image_refusal(
                format!("{code}.not_image"),
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
        Ok((document_id, candidate, before))
    }
}

/// One stable, code-prefixed refusal for an image authoring command.
fn image_refusal(code: impl std::fmt::Display, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// Convert a world-space polygon boundary into the image's pixel-space clip.
///
/// The image plane is spanned by `u`/`v` from `origin`; a point is valid only
/// when it lies on that plane (`|(p − origin) · n| / |n| <= tolerance`), which is
/// the `imageclip.polygon_non_coplanar` refusal. In-plane points are mapped with
/// the exact inverse of the `origin + u·a + v·b` basis (the same `u`/`v` the
/// injective image placement and every clip use), so a polygon vertex has the
/// same pixel coordinate the renderer would compute for it.
///
/// Degenerate boundaries are refused before any transaction: fewer than three
/// distinct points (`imageclip.polygon_points`), a self-intersecting edge ring
/// (`imageclip.polygon_self_intersection`) and a degenerate image basis
/// (`imageclip.degenerate_plane`). A repeated final vertex is dropped; the stored
/// N-gon keeps the engine's open form.
fn polygon_clip(points: &[Point3], origin: Point3, u: Point3, v: Point3) -> CadResult<ImageClip> {
    let normal = cross(u, v);
    let normal_len = length(normal);
    if !normal_len.is_finite() || normal_len <= 1e-12 {
        return Err(image_refusal(
            "imageclip.degenerate_plane",
            "the image basis is degenerate (u × v has no length), so a world-space clip cannot be \
             mapped into pixel space",
        ));
    }
    let tolerance = offset_tolerance();

    // Drop a repeated closing vertex and every consecutive duplicate, so the
    // ring is the open polygon the representation layer stores.
    let mut ring: Vec<Point3> = Vec::with_capacity(points.len());
    for point in points {
        if !is_finite_point(*point) {
            return Err(image_refusal(
                "imageclip.polygon_points",
                "every IMAGECLIP polygon point must be finite",
            ));
        }
        if ring.last().is_some_and(|last| same_point(*last, *point)) {
            continue;
        }
        ring.push(*point);
    }
    if ring.len() >= 2 && same_point(ring[0], ring[ring.len() - 1]) {
        ring.pop();
    }
    if ring.len() < 3 {
        return Err(image_refusal(
            "imageclip.polygon_points",
            format!(
                "IMAGECLIP POLY needs at least three distinct points; got {} after removing \
                 duplicates",
                ring.len()
            ),
        ));
    }
    for point in &ring {
        let offset = Point3 {
            x: point.x - origin.x,
            y: point.y - origin.y,
            z: point.z - origin.z,
        };
        let distance = dot(offset, normal) / normal_len;
        if !distance.is_finite() || distance.abs() > tolerance {
            return Err(image_refusal(
                "imageclip.polygon_non_coplanar",
                format!(
                    "polygon point ({}, {}, {}) is {distance} world units off the image plane \
                     (tolerance {tolerance}); a clip boundary must lie on the image",
                    point.x, point.y, point.z
                ),
            ));
        }
    }

    // In-plane inverse: solve `origin + u·a + v·b = p` with the 2×2 Gram matrix.
    let uu = dot(u, u);
    let uv = dot(u, v);
    let vv = dot(v, v);
    let determinant = uu * vv - uv * uv;
    if !determinant.is_finite() || determinant <= 1e-24 {
        return Err(image_refusal(
            "imageclip.degenerate_plane",
            "the image basis is singular (u and v are parallel), so a world-space clip cannot be \
             mapped into pixel space",
        ));
    }
    let vertices: Vec<[f64; 2]> = ring
        .iter()
        .map(|point| {
            let offset = Point3 {
                x: point.x - origin.x,
                y: point.y - origin.y,
                z: point.z - origin.z,
            };
            let du = dot(offset, u);
            let dv = dot(offset, v);
            [
                (du * vv - dv * uv) / determinant,
                (dv * uu - du * uv) / determinant,
            ]
        })
        .collect();
    if polygon_self_intersects(&vertices) {
        return Err(image_refusal(
            "imageclip.polygon_self_intersection",
            "the IMAGECLIP polygon boundary crosses itself; a simple (non-self-intersecting) ring \
             is required",
        ));
    }
    Ok(ImageClip {
        vertices,
        inside: true,
    })
}

/// The 3-vector cross product.
fn cross(a: Point3, b: Point3) -> Point3 {
    Point3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

/// The dot product.
fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// The Euclidean length.
fn length(a: Point3) -> f64 {
    dot(a, a).sqrt()
}

/// Whether two points coincide within the world tolerance.
fn same_point(a: Point3, b: Point3) -> bool {
    let tolerance = offset_tolerance();
    (a.x - b.x).abs() <= tolerance
        && (a.y - b.y).abs() <= tolerance
        && (a.z - b.z).abs() <= tolerance
}

/// Whether the open pixel-space ring has at least one crossing edge pair.
///
/// Adjacent edges share an endpoint by construction and are skipped (the shared
/// closing edge between the last and the first is skipped too); collinear
/// overlap counts as a crossing, which is the honest answer for a degenerate
/// boundary. The ring is validated to be non-degenerate before this runs.
fn polygon_self_intersects(points: &[[f64; 2]]) -> bool {
    let count = points.len();
    if count < 4 {
        return false;
    }
    for first in 0..count {
        let a = points[first];
        let b = points[(first + 1) % count];
        for second in (first + 1)..count {
            // Skip the edge itself, its immediate neighbour and the wrap pair.
            if second == first || second == (first + 1) % count || (second + 1) % count == first {
                continue;
            }
            let c = points[second];
            let d = points[(second + 1) % count];
            if segments_cross(a, b, c, d) {
                return true;
            }
        }
    }
    false
}

/// Whether two 2D segments cross (proper crossing or collinear overlap).
fn segments_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let o1 = orientation(a, b, c);
    let o2 = orientation(a, b, d);
    let o3 = orientation(c, d, a);
    let o4 = orientation(c, d, b);
    if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
        return true;
    }
    (o1 == 0.0 && on_segment(a, b, c))
        || (o2 == 0.0 && on_segment(a, b, d))
        || (o3 == 0.0 && on_segment(c, d, a))
        || (o4 == 0.0 && on_segment(c, d, b))
}

/// The signed 2D orientation of `c` relative to the direction `a → b`.
fn orientation(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Whether `p` lies inside the axis-aligned box of a collinear segment `a → b`.
fn on_segment(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    p[0] >= a[0].min(b[0])
        && p[0] <= a[0].max(b[0])
        && p[1] >= a[1].min(b[1])
        && p[1] <= a[1].max(b[1])
}
