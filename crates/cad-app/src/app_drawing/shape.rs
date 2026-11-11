//! `SHAPE` creation and `SHP2BLK` conversion.
//!
//! # `SHAPE <name-or-code> <x> <y> [size] [rotation_deg]`
//!
//! One real `SemanticGeometry::Shape` glyph on the active layer, inserted through
//! the shared create funnel ([`Application::commit_new_entity`]: one transaction,
//! one undo step, model space). The payload is
//! [`crate::CommandPayload::Shape`].
//!
//! * **code form** — a positive `u32` token is the font's shape number, the value
//!   the representation layer actually resolves (`FontEngine::shape_glyph`). This
//!   is the form that draws.
//! * **name form** — a non-numeric token is stored as `shape_name` with
//!   `code = 0`. This build's SHX parser keeps shape numbers only (the shape name
//!   is stripped with the glyph bytes), so a name cannot be resolved to a code.
//!   The entity is created honestly and the representation layer keeps reporting
//!   its existing `shape '<name>' is referenced by name; only shape codes are
//!   resolved` partial — the command never invents a code.
//!
//! Font resolution is the payload's explicit key, else the drawing's `Standard`
//! text style font ([`resolve_created_text_style`], the same one TEXT/MTEXT
//! creation uses). Neither present is `shape.font_missing`; this build never
//! bundles or guesses a typeface.
//!
//! Named refusals (all before any transaction, so nothing is written):
//! `shape.payload` · `shape.invalid_point` · `shape.invalid_code` ·
//! `shape.invalid_name` · `shape.invalid_size` · `shape.invalid_rotation` ·
//! `shape.font_missing`.
//!
//! # `SHP2BLK`
//!
//! Converts the selected `SHAPE` rows into block definitions of their **outline
//! polylines** plus one INSERT at each original position, deleting the sources —
//! all in one transaction / one undo step.
//!
//! The outlines come from the process-wide font engine the display uses
//! ([`crate::app_extended_w9_reports::text_explode::outline_fonts`], the same hook
//! `TXTEXP` uses: `cad-app` holds no font bytes). One block is defined **per
//! distinct (font, shape code)**; its members are the unit-size, unrotated glyph
//! outlines at the block origin (`base_point = (0,0,0)`), and every converted
//! shape becomes one `SemanticGeometry::Insert` with
//! `T(position) · R(rotation) · S(size)` — the importer's
//! `T(insertion) · T(−base_point)` composition with a zero base. Grouping by glyph
//! stores each outline once and keeps every source placement exact; a single
//! block for the whole selection could not hold two different glyphs.
//!
//! Block names are generated (font stem + `_SHAPE<code>`) with a numeric suffix on
//! collision, so an existing definition is never overwritten.
//!
//! Named refusals: `shp2blk.payload` · `shp2blk.no_selection` ·
//! `shp2blk.not_addressable` · `shp2blk.not_shape` · `shp2blk.not_model_space` ·
//! `shp2blk.font_unavailable` (no engine installed, or the glyph could not be
//! outlined) · `shp2blk.shape_by_name` · `shp2blk.font_missing` ·
//! `shp2blk.no_outlines` · `shp2blk.block_name` (no free generated name).
//!
//! **Residual boundary**: the produced block definition is not linked back to the
//! SHX font (a later glyph edit is impossible), and a name-referenced SHAPE
//! (`code == 0`) cannot be converted at all because its glyph is unresolvable
//! here (`shp2blk.shape_by_name`).

use super::*;
use crate::app_extended_w9_reports::text_explode::outline_fonts;
use crate::draw_tool::rotation_transform;
use cad_db::{BlockDefinition, BlockMetadata, BlockPatch, BlockRow, DbObject};
use cad_history::{drawing_patch, drawing_patch_deleted, DrawingPatch, DrawingUndoRecord};

/// The characters a generated block name must not contain (the `BLOCK` set).
const ILLEGAL_BLOCK_NAME_CHARS: &[char] = &[
    '<', '>', '/', '\\', '"', ':', ';', '?', '*', '|', '=', '`', '\'',
];

/// The bound on generated-name attempts before `shp2blk.block_name` is refused.
const MAX_GENERATED_NAME_ATTEMPTS: usize = 10_000;

/// One glyph identity: the SHX font key plus the shape code.
type GlyphKey = (String, u32);

/// The unit-size, unrotated outline of one glyph (each run with its closed flag).
struct GlyphOutline {
    key: GlyphKey,
    contours: Vec<(Vec<Point3>, bool)>,
}

/// A `<selector>.<reason>` invalid-input refusal.
fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

/// A `<selector>.<reason>` `Unsupported` refusal.
fn unsupported(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{code}: {detail}"))
}

impl Application {
    /// `SHAPE`: create one SHX shape glyph on the active layer.
    pub(crate) fn create_shape(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Shape(payload) = &command.payload else {
            return Err(refusal(
                "shape.payload",
                "SHAPE needs a Shape payload (name-or-code, x, y, size, rotation, font); the typed \
                 form is `SHAPE <name-or-code> <x> <y> [size] [rotation_deg]`",
            ));
        };
        let shape = payload.as_ref();
        if !is_finite_point(shape.position) {
            return Err(refusal(
                "shape.invalid_point",
                format!(
                    "the SHAPE insertion point {:?} is not finite",
                    shape.position
                ),
            ));
        }
        let name = shape.name.trim();
        if shape.code == 0 {
            if name.is_empty() {
                return Err(refusal(
                    "shape.invalid_code",
                    "SHAPE needs either a positive shape code or a non-empty shape name; got code \
                     0 with no name",
                ));
            }
            if name.contains('\r') || name.contains('\n') {
                return Err(refusal(
                    "shape.invalid_name",
                    "SHAPE shape name must be a single line",
                ));
            }
            let characters = name.chars().count();
            if characters > MAX_TOOL_TEXT_CHARS {
                return Err(refusal(
                    "shape.invalid_name",
                    format!(
                        "SHAPE allows at most {MAX_TOOL_TEXT_CHARS} characters in a shape name, \
                         got {characters}"
                    ),
                ));
            }
        }
        if !shape.size.is_finite() || shape.size <= 0.0 {
            return Err(refusal(
                "shape.invalid_size",
                format!(
                    "SHAPE size must be finite and strictly positive, got {}",
                    shape.size
                ),
            ));
        }
        if !shape.rotation.is_finite() {
            return Err(refusal(
                "shape.invalid_rotation",
                format!("SHAPE rotation must be finite, got {}", shape.rotation),
            ));
        }
        // The explicit payload key wins; otherwise the current text style's font
        // is the same resolution TEXT/MTEXT creation uses. No bundled fallback.
        let explicit = shape
            .font
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string);
        let font = match explicit {
            Some(key) => Some(key),
            None => {
                let document = self
                    .workspace
                    .documents
                    .get(&command.document)
                    .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
                resolve_created_text_style(document.drawing.as_ref()).1
            }
        };
        let Some(font) = font else {
            return Err(refusal(
                "shape.font_missing",
                "SHAPE needs an SHX shape font: the drawing's text style declares none and the \
                 payload carries no explicit font key",
            ));
        };
        let geometry = SemanticGeometry::Shape {
            shape_name: if shape.code == 0 {
                name.to_string()
            } else {
                shape.name.trim().to_string()
            },
            code: shape.code,
            position: shape.position,
            size: shape.size,
            rotation: shape.rotation,
            font: Some(font.clone()),
        };
        let mut outcome = self.commit_new_entity(session, command, geometry, "create shape")?;
        let reference = if shape.code == 0 {
            format!("name '{}' (code unresolved in this build)", name)
        } else {
            format!("code {}", shape.code)
        };
        outcome.diagnostics.push(Diagnostic {
            object: outcome.objects.first().copied(),
            code: "drawing.shape".into(),
            message: format!(
                "SHAPE: created shape {reference} at ({}, {}, {}) size {} rotation {} deg with \
                 font '{}'",
                shape.position.x,
                shape.position.y,
                shape.position.z,
                shape.size,
                shape.rotation.to_degrees(),
                font
            ),
        });
        Ok(outcome)
    }

    /// `SHP2BLK`: convert the selected shapes into outline blocks + INSERTs.
    pub(crate) fn shape_to_block(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(refusal(
                "shp2blk.payload",
                "SHP2BLK takes no payload; the current selection is its input",
            ));
        }
        let document_id = command.document;
        let sources = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            resolve_shape_selection(
                document.drawing.as_ref(),
                document_id,
                session.selection.refs(),
            )?
        };
        // One unit outline per distinct glyph. Every source's own size/rotation
        // becomes part of its INSERT transform below. The outline engine is
        // required only once a glyph really has to be outlined, so the selection
        // refusals above stay stable with no engine installed.
        let mut outlines: Vec<GlyphOutline> = Vec::new();
        let mut groups: Vec<(GlyphKey, Vec<EntityId>)> = Vec::new();
        for source in &sources {
            let SemanticGeometry::Shape {
                shape_name,
                code,
                font,
                ..
            } = &source.geometry
            else {
                return Err(unsupported(
                    "shp2blk.not_shape",
                    "the selection changed after it was resolved",
                ));
            };
            if *code == 0 {
                return Err(unsupported(
                    "shp2blk.shape_by_name",
                    format!(
                        "entity {} references the shape by name '{}'; this build's SHX parser keeps \
                         shape codes, so its glyph cannot be resolved to outlines",
                        source.id.0, shape_name
                    ),
                ));
            }
            let key = font
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .ok_or_else(|| {
                    unsupported(
                        "shp2blk.font_missing",
                        format!(
                            "shape entity {} carries no SHX font key, so its glyph cannot be \
                             outlined",
                            source.id.0
                        ),
                    )
                })?;
            let glyph_key: GlyphKey = (key.to_string(), *code);
            if !outlines.iter().any(|outline| outline.key == glyph_key) {
                let fonts = outline_fonts().ok_or_else(|| {
                    unsupported(
                        "shp2blk.font_unavailable",
                        "no font engine is installed for outline extraction; this build does \
                         not bundle font bytes (the carrier supplies the same engine the display \
                         uses)",
                    )
                })?;
                let polylines = fonts
                    .shape_glyph(key, *code, Point3::default(), 1.0, 0.0)
                    .map_err(|error| {
                        unsupported(
                            "shp2blk.font_unavailable",
                            format!(
                                "shape entity {} (font '{}', code {}) could not be outlined: {error}",
                                source.id.0, key, code
                            ),
                        )
                    })?;
                let mut contours = Vec::with_capacity(polylines.len());
                for polyline in polylines {
                    if let Some(contour) = outline_contour(polyline) {
                        contours.push(contour);
                    }
                }
                if contours.is_empty() {
                    return Err(unsupported(
                        "shp2blk.no_outlines",
                        format!(
                            "the font engine produced no contour for shape code {} in font '{}'",
                            code, key
                        ),
                    ));
                }
                outlines.push(GlyphOutline {
                    key: glyph_key.clone(),
                    contours,
                });
            }
            if let Some((_, members)) = groups.iter_mut().find(|(group, _)| *group == glyph_key) {
                members.push(source.id);
            } else {
                groups.push((glyph_key, vec![source.id]));
            }
        }

        let mut candidate = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .as_ref()
            .clone();
        let drawing = &mut candidate;
        let display_states: Vec<_> = sources
            .iter()
            .map(|source| drawing.entity_display_state(source.id))
            .collect();
        let base_order = next_model_draw_order(drawing);

        let mut block_rows: Vec<BlockRow> = Vec::with_capacity(groups.len());
        let mut block_patches: Vec<BlockPatch> = Vec::with_capacity(groups.len());
        let mut entity_rows: Vec<DbEntity> = Vec::new();
        let mut insert_count = 0usize;
        for (glyph_key, source_ids) in &groups {
            let outline = outlines
                .iter()
                .find(|outline| outline.key == *glyph_key)
                .ok_or_else(|| CadError::Invariant("SHP2BLK lost a glyph's outlines".into()))?;
            let first_source = sources
                .iter()
                .find(|source| source_ids.contains(&source.id))
                .ok_or_else(|| CadError::Invariant("SHP2BLK lost a shape source".into()))?;
            let new_block = drawing.allocate_block_id();
            let name = fresh_block_name(drawing, glyph_key)?;
            let mut member_ids = Vec::with_capacity(outline.contours.len());
            for (index, (points, closed)) in outline.contours.iter().enumerate() {
                let id = drawing.allocate_entity_id();
                member_ids.push(id);
                entity_rows.push(DbEntity {
                    object: DbObject {
                        id: ObjectId(id.0),
                        type_key: "AcDbPolyline".into(),
                        revision: Revision(0),
                        source_handle: None,
                    },
                    id,
                    layer: first_source.layer,
                    space: SpaceId::Block(new_block),
                    geometry: SemanticGeometry::Polyline {
                        points: points.clone(),
                        bulges: Vec::new(),
                        closed: *closed,
                    },
                    draw_order: index as i64,
                });
            }
            let row = BlockRow {
                definition: BlockDefinition {
                    id: new_block,
                    entities: member_ids,
                    dynamic_visibility: None,
                },
                metadata: Some(BlockMetadata {
                    name: name.clone(),
                    base_point: Point3::default(),
                }),
            };
            block_patches.push(BlockPatch {
                id: new_block,
                before: None,
                after: Some(row.clone()),
            });
            block_rows.push(row);

            for source in sources
                .iter()
                .filter(|source| source_ids.contains(&source.id))
            {
                let SemanticGeometry::Shape {
                    position,
                    size,
                    rotation,
                    ..
                } = &source.geometry
                else {
                    return Err(CadError::Invariant(
                        "SHP2BLK source is no longer a shape".into(),
                    ));
                };
                // `T(position) · R(rotation) · S(size)`: the block's unit outline
                // at the origin, placed exactly where the source glyph was.
                let scale = Transform3::scale(*size);
                let rotate = rotation_transform(Point3::default(), *rotation);
                let transform =
                    Transform3::translation(*position).matrix_mul(&rotate.matrix_mul(&scale));
                let id = drawing.allocate_entity_id();
                entity_rows.push(DbEntity {
                    object: DbObject {
                        id: ObjectId(id.0),
                        type_key: "AcDbBlockReference".into(),
                        revision: Revision(0),
                        source_handle: None,
                    },
                    id,
                    layer: source.layer,
                    space: SpaceId::Model,
                    geometry: SemanticGeometry::Insert {
                        block: new_block,
                        transform,
                    },
                    draw_order: base_order + insert_count as i64,
                });
                insert_count += 1;
            }
        }

        let label = "shape to block";
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            for row in &block_rows {
                staged.stage_create_block(row.clone())?;
            }
            for entity in &entity_rows {
                staged.insert_entity(entity.clone())?;
            }
            for source in &sources {
                staged.delete_entity(source.id)?;
            }
            staged.commit()?
        };

        let mut patches: Vec<DrawingPatch> = sources
            .iter()
            .zip(display_states)
            .map(|(source, state)| drawing_patch_deleted(source.id, source.clone(), state))
            .collect();
        patches.extend(
            entity_rows
                .iter()
                .map(|entity| drawing_patch(entity.id, None, Some(entity.clone()))),
        );
        let output_ids: Vec<ObjectId> = entity_rows
            .iter()
            .filter(|entity| matches!(entity.geometry, SemanticGeometry::Insert { .. }))
            .map(|entity| ObjectId(entity.id.0))
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
                patches,
                block_patches,
                layer_patches: Vec::new(),
                style_patches: Vec::new(),
                dim_style_patches: Vec::new(),
                mleader_style_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        // The selected rows are gone; their refs must not survive.
        session.selection = SelectionSet::new();
        Ok(CommandOutcome {
            objects: output_ids,
            changes: Some(Arc::new(change_set)),
            diagnostics: vec![Diagnostic {
                object: None,
                code: "shp2blk.created".into(),
                message: format!(
                    "SHP2BLK: {} shape(s) converted into {} block definition(s) and {} INSERT(s); \
                     the source shapes were deleted (one transaction)",
                    sources.len(),
                    block_rows.len(),
                    insert_count
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// Resolve the selection to model-space SHAPE rows, with stable refusal codes.
///
/// Every refusal keeps its `shp2blk.*` prefix; an empty selection, a foreign or
/// non-addressable ref, a missing row, a non-SHAPE type and a paper-space/block
/// member are all named. Duplicate refs collapse to one row.
fn resolve_shape_selection(
    drawing: &DrawingDatabase,
    document_id: DocumentId,
    refs: &[SelectionRef],
) -> CadResult<Vec<DbEntity>> {
    if refs.is_empty() {
        return Err(refusal(
            "shp2blk.no_selection",
            "SHP2BLK needs at least one selected SHAPE entity",
        ));
    }
    let mut seen: Vec<EntityId> = Vec::new();
    let mut sources = Vec::with_capacity(refs.len());
    for reference in refs {
        if reference.document != document_id
            || !reference.instance.0.is_empty()
            || reference.sub_element.is_some()
        {
            return Err(unsupported(
                "shp2blk.not_addressable",
                format!(
                    "entity {} is not a directly addressable model entity in this document",
                    reference.entity.0
                ),
            ));
        }
        if seen.contains(&reference.entity) {
            continue;
        }
        seen.push(reference.entity);
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            refusal(
                "shp2blk.not_addressable",
                format!(
                    "entity {} does not exist in this drawing",
                    reference.entity.0
                ),
            )
        })?;
        if !matches!(entity.geometry, SemanticGeometry::Shape { .. }) {
            return Err(unsupported(
                "shp2blk.not_shape",
                format!(
                    "SHP2BLK converts SHAPE entities only; entity {} is a {}",
                    entity.id.0,
                    class_name(&entity.geometry)
                ),
            ));
        }
        if entity.space != SpaceId::Model {
            return Err(unsupported(
                "shp2blk.not_model_space",
                format!(
                    "entity {} is not in model space; paper-space and block-member shapes cannot \
                     be converted",
                    entity.id.0
                ),
            ));
        }
        sources.push(entity);
    }
    Ok(sources)
}

/// Normalise one engine contour: a repeated closing point is dropped and marked
/// closed; a run shorter than two points is not a polyline and is dropped.
fn outline_contour(points: Vec<Point3>) -> Option<(Vec<Point3>, bool)> {
    if points.len() < 2 {
        return None;
    }
    let closed = polyline_closed(&points);
    let points = if closed {
        points[..points.len() - 1].to_vec()
    } else {
        points
    };
    (points.len() >= 2).then_some((points, closed))
}

/// Whether one point run repeats its first point at the end (the closed rule).
fn polyline_closed(points: &[Point3]) -> bool {
    if points.len() < 3 {
        return false;
    }
    let first = points[0];
    let last = points[points.len() - 1];
    (first.x - last.x).abs() <= 1e-9
        && (first.y - last.y).abs() <= 1e-9
        && (first.z - last.z).abs() <= 1e-9
}

/// A free generated block name for one glyph, never overwriting an existing one.
fn fresh_block_name(drawing: &DrawingDatabase, glyph: &GlyphKey) -> CadResult<String> {
    let (font, code) = glyph;
    let stem = block_name_stem(font, *code);
    for attempt in 0..MAX_GENERATED_NAME_ATTEMPTS {
        let candidate = if attempt == 0 {
            stem.clone()
        } else {
            format!("{stem}_{attempt}")
        };
        if drawing.block_by_name(&candidate).is_none() {
            return Ok(candidate);
        }
    }
    Err(unsupported(
        "shp2blk.block_name",
        format!(
            "could not generate a free block name from '{stem}': every candidate through \
             {MAX_GENERATED_NAME_ATTEMPTS} attempts collides with an existing definition"
        ),
    ))
}

/// The name stem for a glyph: the font stem plus `_SHAPE<code>`, sanitised.
fn block_name_stem(font: &str, code: u32) -> String {
    let font_stem = font
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(font)
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(font);
    let mut stem: String = format!("{font_stem}_SHAPE{code}")
        .chars()
        .map(|character| {
            if ILLEGAL_BLOCK_NAME_CHARS.contains(&character) || character.is_whitespace() {
                '_'
            } else {
                character
            }
        })
        .collect();
    if stem.is_empty() {
        stem = format!("SHAPE{code}");
    }
    // The block-name cap is the shared text cap; truncate on a char boundary.
    if stem.chars().count() > MAX_TOOL_TEXT_CHARS {
        stem = stem.chars().take(MAX_TOOL_TEXT_CHARS).collect();
    }
    stem
}

/// The next free draw order above every model-space row (the EXPLODE rule).
fn next_model_draw_order(drawing: &DrawingDatabase) -> i64 {
    drawing
        .model_space()
        .iter()
        .map(|entity| entity.draw_order)
        .max()
        .map(|max| max.saturating_add(1))
        .unwrap_or(0)
}
