//! `TXTEXP`: explode selected `TEXT`/`MTEXT` into their font outlines.
//!
//! ## What is produced
//!
//! Every selected text entity is shaped by the **same** font engine the display
//! uses ([`cad_representation::FontEngine`]) and every glyph contour becomes one
//! `LWPOLYLINE` on the source's own layer and space:
//!
//! * a **closed** contour (an sfnt outline, whose last point repeats the first)
//!   becomes a closed polyline with the repeated point dropped, so the closing
//!   segment is not a zero-length tail;
//! * an **open** stroke (an SHX shape font draws pen linework, not filled
//!   contours) stays an open polyline — closing a stroke would invent an edge the
//!   font never drew, and the outcome says so (`txtexp.open_contours`);
//! * the sources are deleted and the polylines inserted in **one transaction /
//!   one undo step** (the EXPLODE shape: delete patches carry the pruned display
//!   state, insert patches carry the new rows).
//!
//! MTEXT is exploded through the same call: the engine parses the inline codes
//! itself (`\P`, `\f`, `\H`, stacking, …). The one MTEXT facet the creation path
//! cannot reproduce is the **width boundary**: `outline` never wraps, so a
//! wrapped MTEXT explodes into unwrapped lines and the outcome reports
//! `txtexp.mtext_no_wrap` instead of silently pretending the wrap was applied.
//!
//! ## Font faces
//!
//! `cad-app` holds **no font bytes** by design — the host owns the font engine and
//! hands the display an `Arc<FontEngine>`. TXTEXP needs the same faces, so it
//! reads one optional process-wide engine installed through
//! [`install_outline_fonts`]. The installation point is crate-internal today (the
//! `lib.rs` re-export a carrier would call is a residual gap, recorded in the
//! handoff): the in-crate contract tests install the repository's open font, and
//! any crate-internal carrier adapter can call the same hook. With nothing
//! installed, or with a text whose font (and fallback chain) holds no usable face,
//! the command refuses `txtexp.font_unavailable` and writes nothing: it never
//! substitutes a bundled face, because that would silently change the drawing's
//! typeface.
//!
//! ## Refusals (named, nothing written, no transaction)
//!
//! * `txtexp.payload` — the command takes no payload;
//! * `txtexp.selection` / `txtexp.not_addressable` — empty selection, a ref from
//!   another document, a block-instance member or a sub-element ref;
//! * `txtexp.not_text` — any selected entity is not `TEXT`/`MTEXT`;
//! * `txtexp.font_unavailable` — no engine installed, or the font/fallback chain
//!   holds no decodable face;
//! * `txtexp.no_outlines` — the engine produced no contour for the string;
//! * `txtexp.too_many` — the explosion would exceed [`MAX_TXTEXP_ENTITIES`].
//!
//! Viewer mode is refused up front with [`CadError::PermissionDenied`]: the
//! command writes entity rows, and this build's catalogue flag for `TXTEXP`
//! predates it being a writer.

use super::*;
use crate::app_drawing::{class_name, next_transaction_id};
use crate::app_reports_ext::diagnostic;
use cad_db::{DbEntity, DbObject};
use cad_domain::EntityId;
use cad_history::{drawing_patch, drawing_patch_deleted, DrawingPatch, DrawingUndoRecord};
use cad_representation::FontEngine;
use std::sync::{Arc, OnceLock};

/// The bound on produced polylines for one `TXTEXP` call.
///
/// A long MTEXT in a font with many contours per glyph can otherwise allocate an
/// unbounded number of rows. 20 000 is far above any hand-authored string this
/// build's `MAX_TOOL_TEXT_CHARS` limit allows, so the cap only catches a corrupt
/// or hostile document.
pub(crate) const MAX_TXTEXP_ENTITIES: usize = 20_000;

/// The one optional font engine the outline path may use.
///
/// The host installs it once per process; `cad-app` never loads font bytes
/// itself (no file I/O, no bundled face). The first installation wins, so a
/// second one cannot silently change the typeface mid-session.
static OUTLINE_FONTS: OnceLock<Arc<FontEngine>> = OnceLock::new();

/// Install the font engine `TXTEXP` outlines with.
///
/// Returns `false` when an engine was already installed (the first one stays
/// authoritative). A carrier calls this from the same place it hands the render
/// session its fonts; a test calls it with the repository's open font. Nothing in
/// the crate's non-test code calls it today (the `lib.rs` re-export a carrier
/// adapter needs is a recorded residual), hence the `dead_code` allowance.
#[allow(dead_code)]
pub fn install_outline_fonts(fonts: Arc<FontEngine>) -> bool {
    OUTLINE_FONTS.set(fonts).is_ok()
}

/// The installed engine, if the carrier supplied one.
pub fn outline_fonts() -> Option<&'static Arc<FontEngine>> {
    OUTLINE_FONTS.get()
}

/// One entity produced by an explosion, before it gets an id, layer and order.
pub(crate) struct ProducedEntity {
    pub(crate) geometry: SemanticGeometry,
    pub(crate) layer: LayerId,
    pub(crate) space: SpaceId,
}

impl Application {
    /// `TXTEXP`: explode the selected `TEXT`/`MTEXT` rows into outline polylines.
    pub(crate) fn text_explode(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        if !matches!(command.payload, CommandPayload::None) {
            return Err(CadError::InvalidInput(
                "txtexp.payload: TXTEXP takes no payload; the current selection is its input"
                    .into(),
            ));
        }
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let drawing = document.drawing.as_ref();
        let sources =
            resolve_text_selection(drawing, document_id, session.selection.refs(), "txtexp")?;
        let fonts = outline_fonts().ok_or_else(|| {
            unsupported(
                "txtexp",
                "font_unavailable",
                "no font engine is installed for outline extraction; this build does not bundle \
                 font faces (the carrier supplies the same engine the display uses)",
            )
        })?;

        let mut produced: Vec<ProducedEntity> = Vec::new();
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        let mut open_contours = 0usize;
        let mut wrapped_mtext = 0usize;
        for source in &sources {
            let (mut rows, open) = outline_text_entity(fonts, source)?;
            open_contours += open;
            if let SemanticGeometry::Text {
                wrap_width: Some(_),
                ..
            } = &source.geometry
            {
                wrapped_mtext += 1;
            }
            produced.append(&mut rows);
            if produced.len() > MAX_TXTEXP_ENTITIES {
                return Err(unsupported(
                    "txtexp",
                    "too_many",
                    format!(
                        "the explosion would produce more than {MAX_TXTEXP_ENTITIES} polylines; \
                         explode the text in smaller pieces"
                    ),
                ));
            }
        }
        if produced.is_empty() {
            return Err(unsupported(
                "txtexp",
                "no_outlines",
                "the font engine produced no contour for the selected text; nothing was written",
            ));
        }
        let produced_count = produced.len();
        let source_count = sources.len();
        let mut outcome = replace_entities(
            self,
            command,
            sources,
            produced,
            "explode text",
            "drawing.txtexp",
        )?;
        diagnostics.append(&mut outcome.diagnostics);
        diagnostics.push(diagnostic(
            "txtexp.partial",
            format!(
                "TXTEXP: {produced_count} outline polyline(s) from {source_count} text row(s); the \
                 sources are deleted and the outlines are real geometry, so the text is no longer \
                 editable as text and its style/font linkage is gone"
            ),
        ));
        if open_contours > 0 {
            diagnostics.push(diagnostic(
                "txtexp.open_contours",
                format!(
                    "TXTEXP: {open_contours} contour(s) were open strokes (an SHX shape font draws \
                     pen linework); they stay open polylines instead of being closed into edges \
                     the font never drew"
                ),
            ));
        }
        if wrapped_mtext > 0 {
            diagnostics.push(diagnostic(
                "txtexp.mtext_no_wrap",
                format!(
                    "TXTEXP: {wrapped_mtext} MTEXT row(s) carry a width boundary; the outline path \
                     never wraps, so those lines are placed unwrapped"
                ),
            ));
        }
        outcome.diagnostics = diagnostics;
        Ok(outcome)
    }
}

/// Resolve the current selection to text rows, refusing every unsupported shape.
fn resolve_text_selection(
    drawing: &DrawingDatabase,
    document_id: DocumentId,
    refs: &[SelectionRef],
    prefix: &str,
) -> CadResult<Vec<DbEntity>> {
    if refs.is_empty() {
        return Err(invalid(
            prefix,
            "selection",
            "the current selection is empty; select TEXT or MTEXT first",
        ));
    }
    let mut seen: Vec<EntityId> = Vec::new();
    let mut entities: Vec<DbEntity> = Vec::new();
    for reference in refs {
        if reference.document != document_id {
            return Err(invalid(
                prefix,
                "not_addressable",
                "the selection contains an entity from another document",
            ));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(unsupported(
                prefix,
                "not_addressable",
                "block-instance members and sub-element references cannot be exploded",
            ));
        }
        if seen.contains(&reference.entity) {
            continue;
        }
        seen.push(reference.entity);
        let entity = drawing.entity(reference.entity).cloned().ok_or_else(|| {
            invalid(
                prefix,
                "not_addressable",
                format!("entity {} is not in the drawing", reference.entity.0),
            )
        })?;
        if !matches!(entity.geometry, SemanticGeometry::Text { .. }) {
            return Err(unsupported(
                prefix,
                "not_text",
                format!(
                    "TXTEXP supports TEXT and MTEXT only; entity {} is {}",
                    entity.id.0,
                    class_name(&entity.geometry)
                ),
            ));
        }
        entities.push(entity);
    }
    Ok(entities)
}

/// Outline one text row into closed/open polylines on its own layer and space.
///
/// Returns the produced rows plus the number of contours that were **open**
/// strokes (an SHX face), so the caller can report the divergence once.
fn outline_text_entity(
    fonts: &FontEngine,
    source: &DbEntity,
) -> CadResult<(Vec<ProducedEntity>, usize)> {
    let SemanticGeometry::Text {
        text,
        position,
        height,
        rotation,
        font,
        h_align,
        v_align,
        ..
    } = &source.geometry
    else {
        return Err(unsupported(
            "txtexp",
            "not_text",
            "the outline path was handed a non-text entity",
        ));
    };
    let key = font.as_deref().unwrap_or("");
    let contours = fonts
        .outline(key, text, *position, *height, *rotation, *h_align, *v_align)
        .map_err(|error| {
            unsupported(
                "txtexp",
                "font_unavailable",
                format!(
                    "entity {} could not be outlined with font {}: {error}",
                    source.id.0,
                    if key.is_empty() {
                        "(style default)"
                    } else {
                        key
                    }
                ),
            )
        })?;
    let mut produced = Vec::with_capacity(contours.len());
    let mut open = 0usize;
    for contour in contours {
        let Some((points, closed)) = contour_points(&contour) else {
            continue;
        };
        if !closed {
            open += 1;
        }
        produced.push(ProducedEntity {
            geometry: SemanticGeometry::Polyline {
                points,
                bulges: Vec::new(),
                closed,
            },
            layer: source.layer,
            space: source.space.clone(),
        });
    }
    Ok((produced, open))
}

/// Normalise one engine contour: drop the repeated closing point of a closed
/// loop, and refuse a contour that cannot hold a polyline at all.
///
/// The engine emits a closed sfnt contour with its **first point repeated at the
/// end** (`FontEngine`'s outline builder pushes it in `close`). Storing that point
/// again on a `closed` polyline would add a zero-length segment, so it is dropped
/// here; an open SHX stroke has distinct ends and stays open.
fn contour_points(contour: &[Point3]) -> Option<(Vec<Point3>, bool)> {
    const EPS: f64 = 1e-9;
    if contour.len() < 2 {
        return None;
    }
    let first = contour[0];
    let last = contour[contour.len() - 1];
    let closed = (first.x - last.x).abs() <= EPS
        && (first.y - last.y).abs() <= EPS
        && (first.z - last.z).abs() <= EPS;
    let points: Vec<Point3> = if closed {
        contour[..contour.len() - 1].to_vec()
    } else {
        contour.to_vec()
    };
    (points.len() >= 2).then_some((points, closed))
}

/// Delete `sources` and insert `produced` in **one transaction / one undo step**.
///
/// This is the EXPLODE write shape: the delete patches carry the pruned display
/// state (so undo restores render attributes and dynamic-visibility membership)
/// and the insert patches carry the new rows. A refusal never reaches this
/// function, so nothing is staged unless the whole command can be committed.
pub(crate) fn replace_entities(
    app: &mut Application,
    command: &Command,
    sources: Vec<DbEntity>,
    produced: Vec<ProducedEntity>,
    label: &str,
    report_code: &str,
) -> CadResult<CommandOutcome> {
    if produced.is_empty() {
        return Err(unsupported(
            "txtexp",
            "no_outlines",
            "nothing to insert; an explosion never succeeds empty",
        ));
    }
    let document_id = command.document;
    let document = app
        .workspace
        .documents
        .get(&document_id)
        .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
    let mut candidate = document.drawing.as_ref().clone();
    let drawing = &mut candidate;
    let display_states: Vec<_> = sources
        .iter()
        .map(|source| drawing.entity_display_state(source.id))
        .collect();
    let base_order = next_draw_order(drawing);
    let mut children: Vec<(EntityId, DbEntity)> = Vec::with_capacity(produced.len());
    for (index, row) in produced.into_iter().enumerate() {
        let id = drawing.allocate_entity_id();
        children.push((
            id,
            DbEntity {
                object: DbObject {
                    id: ObjectId(id.0),
                    type_key: class_name(&row.geometry).to_string(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id,
                layer: row.layer,
                space: row.space,
                geometry: row.geometry,
                draw_order: base_order + index as i64,
            },
        ));
    }
    let transaction = next_transaction_id();
    let change_set = {
        let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
        for source in &sources {
            staged.delete_entity(source.id)?;
        }
        for (_, child) in &children {
            staged.insert_entity(child.clone())?;
        }
        staged.commit()?
    };
    let mut patches: Vec<DrawingPatch> = sources
        .iter()
        .zip(display_states)
        .map(|(source, state)| drawing_patch_deleted(source.id, source.clone(), state))
        .collect();
    patches.extend(
        children
            .iter()
            .map(|(id, child)| drawing_patch(*id, None, Some(child.clone()))),
    );
    app.publish_candidate(
        document_id,
        candidate,
        DrawingUndoRecord {
            transaction: change_set.transaction,
            label: label.to_string(),
            patches,
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
        objects: children.iter().map(|(id, _)| ObjectId(id.0)).collect(),
        changes: Some(Arc::new(change_set)),
        diagnostics: vec![diagnostic(
            report_code,
            format!(
                "{}: {} entities produced from {} sources (sources deleted)",
                label.to_ascii_uppercase(),
                children.len(),
                sources.len()
            ),
        )],
        measurement: None,
        point_report: None,
    })
}

/// The next free draw order above every model-space row (the EXPLODE rule).
fn next_draw_order(drawing: &DrawingDatabase) -> i64 {
    drawing
        .model_space()
        .iter()
        .map(|entity| entity.draw_order)
        .max()
        .map(|max| max.saturating_add(1))
        .unwrap_or(0)
}

/// A `<prefix>.<code>` invalid-input refusal.
pub(crate) fn invalid(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{prefix}.{code}: {detail}"))
}

/// A `<prefix>.<code>` unsupported refusal.
pub(crate) fn unsupported(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::Unsupported(format!("{prefix}.{code}: {detail}"))
}
