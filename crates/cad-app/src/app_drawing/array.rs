//! ARRAYRECT / ARRAYPOLAR edit commands (ADR 0016 decision D).
//!
//! Both commands insert **independent copies** of the current selection using the
//! existing N-copies-in-one-transaction shape (`copy_entities`/`mirror_selection`):
//! every copy is a fresh entity, the sources are **retained**, and the whole array
//! is one transaction with one undo step. There is **no** second insertion path and
//! no associativity (AutoCAD's associative array model is NOT IMPLEMENTED — the
//! project has no associative object/grip mechanism).
//!
//! The placement maths are our **declared** rules (AutoCAD's exact base-point and
//! polar-placement rules are unverified); they are documented here and pinned by
//! the contract tests.
//!
//! Every input is validated before any write, so a refusal changes nothing (the
//! database transaction is all-or-nothing and no history is recorded).

use super::*;

use cad_db::BoundsAccumulator;

use crate::draw_tool::{rotation_transform, ToolParameter, ToolValue};

/// The explicit ARRAY entity cap (ADR 0016 decision D, the `MAXARRAY` order of
/// magnitude): the total number of copies an array may produce.
///
/// A request that would produce more than this is **refused**, never truncated.
pub const MAX_ARRAY_ENTITIES: usize = 100_000;

/// The minimum centre→base extent for an ARRAYPOLAR direction.
///
/// A centre coincident with the base point has no direction; it is a degenerate
/// centre and is refused rather than fabricating an angle.
const MIN_ARRAY_RADIUS: f64 = 1e-12;

impl Application {
    /// ARRAYRECT: insert independent copies of the selection on a rectangular grid.
    ///
    /// Our declared geometry rule (AutoCAD's exact rule is unverified): the base
    /// point is the selection's **bounding-box lower-left**, and cell `(row,
    /// column)` translates the source by `(column·columnSpacing, row·rowSpacing)`
    /// — spacing is the **distance between base points**, and the array grows **up
    /// and to the right**. (The offsets are measured between base points, so the
    /// numeric result is independent of where the declared base sits.)
    ///
    /// **Copy count** (ADR 0016 decision D): `rows × columns` is the total
    /// **including the source object**, which stays where it is and is not
    /// duplicated, so the array inserts `rows × columns − 1` copies. The source's
    /// own cell `(0, 0)` is skipped. A 1×1 "array" therefore produces **0 copies**;
    /// that is an explicit diagnostic, never a silent success.
    ///
    /// Refusals (nothing written): non-integer or `< 1` rows/columns; non-finite or
    /// negative spacing; an empty selection; source geometry that cannot be
    /// translated (the existing `Unsupported` surface); and a total produced copy
    /// count above [`MAX_ARRAY_ENTITIES`].
    pub(crate) fn array_rect(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ArrayRect { refs } = &command.payload else {
            return Err(CadError::InvalidInput(
                "ArrayRectangular needs an ArrayRect { refs } payload".into(),
            ));
        };
        let rows = array_count(session, ToolParameter::ArrayRows, "ARRAYRECT")?;
        let columns = array_count(session, ToolParameter::ArrayColumns, "ARRAYRECT")?;
        let row_spacing = array_spacing(session, ToolParameter::ArrayRowSpacing, "ARRAYRECT")?;
        let column_spacing =
            array_spacing(session, ToolParameter::ArrayColumnSpacing, "ARRAYRECT")?;

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        let (sources, mut diagnostics) = Self::resolve_selection(
            drawing,
            document_id,
            refs,
            "array",
            "ARRAYRECT",
            "array",
            false,
        )?;

        // `rows * columns` includes the source's own cell, so the copies are one
        // fewer per source.
        let copies_per_source = rows * columns - 1.0;
        let produced = copies_per_source * sources.len() as f64;
        check_array_cap(produced, "ARRAYRECT")?;

        // A 1×1 array produces no copies: the source already occupies the only
        // cell. 0 is a result, reported explicitly, never a silent success.
        if copies_per_source <= 0.0 {
            diagnostics.push(array_no_copies_diagnostic("ARRAYRECT"));
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics,
                measurement: None,
            });
        }

        // Refuse the whole command if any source geometry cannot be translated
        // (the existing `Unsupported` surface), never dropping an entity silently.
        let probe = Transform3::translation(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        for source in &sources {
            transform_geometry(&source.geometry, &probe)
                .map_err(|error| array_transform_refusal("ARRAYRECT", source, error))?;
            push_array_style_diagnostic(drawing, source, &mut diagnostics);
        }

        let base_order = next_draw_order(drawing);
        let mut copies: Vec<(EntityId, DbEntity)> = Vec::with_capacity(produced as usize);
        let mut index = 0usize;
        // Row-major over the grid; the source's own cell (0, 0) is skipped so
        // `rows × columns − 1` copies are inserted.
        for row in 0..rows as usize {
            for column in 0..columns as usize {
                if row == 0 && column == 0 {
                    continue;
                }
                let transform = Transform3::translation(Point3 {
                    x: column as f64 * column_spacing,
                    y: row as f64 * row_spacing,
                    z: 0.0,
                });
                for source in &sources {
                    copies.push(array_copy(
                        drawing,
                        source,
                        &transform,
                        base_order + index as i64,
                    )?);
                    index += 1;
                }
            }
        }

        let label = "array rect";
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            for (_, copy) in &copies {
                staged.insert_entity(copy.clone())?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = copies
            .iter()
            .map(|(id, copy)| drawing_patch(*id, None, Some(copy.clone())))
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
                patches,
                merge_key: None,
            },
        )?;

        diagnostics.push(array_diagnostic("ARRAYRECT", copies.len()));
        Ok(CommandOutcome {
            objects: copies.iter().map(|(id, _)| ObjectId(id.0)).collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
        })
    }

    /// ARRAYPOLAR: insert independent copies of the selection around `center`.
    ///
    /// Our declared, **unverified** placement rule (the contract tests pin it):
    /// item `i` (`i` in `0..items`) is placed at angle `i·fillAngle / items`,
    /// measured counter-clockwise from the base point's direction (the base point
    /// is the selection's **bounding-box lower-left**). With `rotateItems = true`
    /// each copy is **rotated about the centre** by that angle; with `false` each
    /// copy is **translated only** to its angular position, its own orientation
    /// unchanged.
    ///
    /// **Copy count** (ADR 0016 decision D): `items` is the total **including the
    /// source object**, which sits at its own direction and is not duplicated, so
    /// the array inserts `items − 1` copies. The item whose angle offset is 0 (item
    /// 0) is skipped. A 1-item "array" therefore produces **0 copies**; that is an
    /// explicit diagnostic, never a silent success.
    ///
    /// Refusals (nothing written): a non-finite centre; a centre coincident with
    /// the base point (no direction); non-integer or `< 1` items; a fill angle
    /// outside `0 < |angle| <= 360`; an empty selection; source geometry that
    /// cannot be transformed (the existing `Unsupported` surface); and a total
    /// produced copy count above [`MAX_ARRAY_ENTITIES`].
    pub(crate) fn array_polar(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ArrayPolar { refs, center } = &command.payload else {
            return Err(CadError::InvalidInput(
                "ArrayPolar needs an ArrayPolar { refs, center } payload".into(),
            ));
        };
        let center = *center;
        if !is_finite_point(center) {
            return Err(CadError::InvalidInput(
                "ARRAYPOLAR centre must be finite".into(),
            ));
        }
        let items = array_count(session, ToolParameter::ArrayItems, "ARRAYPOLAR")?;
        let fill_angle = array_fill_angle(session)?;
        let rotate_items = array_flag(session, ToolParameter::ArrayRotateItems);

        let document_id = command.document;
        let document = self
            .workspace
            .documents
            .get_mut(&document_id)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
        let mut candidate = document.drawing.as_ref().clone();
        let drawing = &mut candidate;

        let (sources, mut diagnostics) = Self::resolve_selection(
            drawing,
            document_id,
            refs,
            "array",
            "ARRAYPOLAR",
            "array",
            false,
        )?;

        // The base point is the selection's bounding-box lower-left. Without a
        // computable box there is no direction to place around, so the command is
        // refused rather than fabricating one.
        let base = selection_lower_left(&sources, "ARRAYPOLAR")?;
        let dx = base.x - center.x;
        let dy = base.y - center.y;
        let radius = dx.hypot(dy);
        if !radius.is_finite() || radius <= MIN_ARRAY_RADIUS {
            return Err(CadError::InvalidInput(
                "ARRAYPOLAR centre must differ from the selection base point".into(),
            ));
        }
        let base_angle = dy.atan2(dx);
        let fill_radians = fill_angle.to_radians();

        // `items` includes the source's own position (item 0), so the copies are
        // one fewer per source.
        let copies_per_source = items - 1.0;
        let produced = copies_per_source * sources.len() as f64;
        check_array_cap(produced, "ARRAYPOLAR")?;

        // A 1-item array produces no copies: the source already sits at the only
        // angle. 0 is a result, reported explicitly, never a silent success.
        if copies_per_source <= 0.0 {
            diagnostics.push(array_no_copies_diagnostic("ARRAYPOLAR"));
            return Ok(CommandOutcome {
                objects: Vec::new(),
                changes: None,
                diagnostics,
                measurement: None,
            });
        }

        // Refuse the whole command if any source geometry cannot take the copy
        // transform (a proper rotation for rotateItems, a translation otherwise).
        let probe = if rotate_items {
            rotation_transform(center, 0.0)
        } else {
            Transform3::translation(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            })
        };
        for source in &sources {
            transform_geometry(&source.geometry, &probe)
                .map_err(|error| array_transform_refusal("ARRAYPOLAR", source, error))?;
            push_array_style_diagnostic(drawing, source, &mut diagnostics);
        }

        let base_order = next_draw_order(drawing);
        let mut copies: Vec<(EntityId, DbEntity)> = Vec::with_capacity(produced as usize);
        let mut index = 0usize;
        // Item 0 (angle offset 0) is the source's own position and is skipped, so
        // `items − 1` copies are inserted.
        for item in 1..items as usize {
            let delta = item as f64 * fill_radians / items;
            let transform = if rotate_items {
                rotation_transform(center, delta)
            } else {
                let angle = base_angle + delta;
                let target = Point3 {
                    x: center.x + radius * angle.cos(),
                    y: center.y + radius * angle.sin(),
                    z: base.z,
                };
                Transform3::translation(Point3 {
                    x: target.x - base.x,
                    y: target.y - base.y,
                    z: 0.0,
                })
            };
            for source in &sources {
                copies.push(array_copy(
                    drawing,
                    source,
                    &transform,
                    base_order + index as i64,
                )?);
                index += 1;
            }
        }

        let label = "array polar";
        let transaction = next_transaction_id();
        let change_set = {
            let mut staged = drawing.begin_drawing_transaction(label, transaction)?;
            for (_, copy) in &copies {
                staged.insert_entity(copy.clone())?;
            }
            staged.commit()?
        };
        let patches: Vec<DrawingPatch> = copies
            .iter()
            .map(|(id, copy)| drawing_patch(*id, None, Some(copy.clone())))
            .collect();
        self.publish_candidate(
            document_id,
            candidate,
            DrawingUndoRecord {
                transaction: change_set.transaction,
                label: label.into(),
                patches,
                merge_key: None,
            },
        )?;

        diagnostics.push(array_diagnostic("ARRAYPOLAR", copies.len()));
        Ok(CommandOutcome {
            objects: copies.iter().map(|(id, _)| ObjectId(id.0)).collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
        })
    }
}

/// Build one fresh-id copy of `source` under `transform`, preserving its layer,
/// space and type key.
fn array_copy(
    drawing: &mut DrawingDatabase,
    source: &DbEntity,
    transform: &Transform3,
    draw_order: i64,
) -> CadResult<(EntityId, DbEntity)> {
    let new_id = drawing.allocate_entity_id();
    let geometry = transform_geometry(&source.geometry, transform)?;
    let copy = DbEntity {
        object: cad_db::DbObject {
            id: ObjectId(new_id.0),
            type_key: source.object.type_key.clone(),
            revision: Revision(0),
            source_handle: None,
        },
        id: new_id,
        layer: source.layer,
        space: source.space.clone(),
        geometry,
        draw_order,
    };
    Ok((new_id, copy))
}

/// The next free draw order above every model-space entity.
fn next_draw_order(drawing: &DrawingDatabase) -> i64 {
    drawing
        .model_space()
        .iter()
        .map(|entity| entity.draw_order)
        .max()
        .map(|max| max.saturating_add(1))
        .unwrap_or(0)
}

/// The selection's bounding-box lower-left, or an explicit `Unsupported` when no
/// selected entity contributes a computable box.
fn selection_lower_left(sources: &[DbEntity], operation: &str) -> CadResult<Point3> {
    let mut bounds = BoundsAccumulator::new();
    for source in sources {
        bounds.add_geometry(&source.geometry);
    }
    let (min, _max) = bounds.finish().ok_or_else(|| {
        CadError::Unsupported(format!(
            "{operation} cannot determine the selection bounding box"
        ))
    })?;
    Ok(min)
}

/// The row/column/item count of a session parameter, defensively validated as a
/// finite integer `>= 1` (the channel is the authority, but the command never
/// trusts an unvalidated value).
fn array_count(session: &SessionState, parameter: ToolParameter, command: &str) -> CadResult<f64> {
    let value = session.tool_parameter(parameter).number().unwrap_or(0.0);
    if value.is_finite() && value.fract() == 0.0 && value >= 1.0 {
        Ok(value)
    } else {
        Err(CadError::InvalidInput(format!(
            "{command} parameter '{}' must be an integer >= 1, got {value}",
            parameter.key()
        )))
    }
}

/// The row/column spacing of a session parameter: finite and **not negative**.
///
/// The channel accepts any finite spacing (including negative); the command
/// refuses a negative spacing rather than inventing the unverified reversal.
fn array_spacing(
    session: &SessionState,
    parameter: ToolParameter,
    command: &str,
) -> CadResult<f64> {
    let value = session.tool_parameter(parameter).number().unwrap_or(0.0);
    if !value.is_finite() {
        return Err(CadError::InvalidInput(format!(
            "{command} parameter '{}' must be finite, got {value}",
            parameter.key()
        )));
    }
    if value < 0.0 {
        return Err(CadError::InvalidInput(format!(
            "{command} parameter '{}' must not be negative, got {value}",
            parameter.key()
        )));
    }
    Ok(value)
}

/// The ARRAYPOLAR fill angle in degrees.
///
/// The legal range (ADR 0016 decision A, mirrored by the channel) is
/// `0 < |angle| <= 360`: `360`/`-360` (the full-circle default) are valid, while
/// `0`, `|angle| > 360` and non-finite values are refused with no implicit wrap.
fn array_fill_angle(session: &SessionState) -> CadResult<f64> {
    let value = session
        .tool_parameter(ToolParameter::ArrayFillAngle)
        .number()
        .unwrap_or(0.0);
    if value.is_finite() && value != 0.0 && value.abs() <= 360.0 {
        Ok(value)
    } else {
        Err(CadError::InvalidInput(format!(
            "ARRAYPOLAR fill angle must be finite, non-zero and within a single turn \
             (0 < |angle| <= 360), got {value}"
        )))
    }
}

/// The boolean value of a session flag parameter (the channel validates the kind,
/// so the numeric arm is unreachable and falls back to `false`).
fn array_flag(session: &SessionState, parameter: ToolParameter) -> bool {
    match session.tool_parameter(parameter) {
        ToolValue::Flag(value) => value,
        ToolValue::Number(_) => false,
    }
}

/// Refuse a total produced copy count above [`MAX_ARRAY_ENTITIES`], never truncate.
fn check_array_cap(produced: f64, command: &str) -> CadResult<()> {
    if produced.is_finite() && produced <= MAX_ARRAY_ENTITIES as f64 {
        Ok(())
    } else {
        Err(CadError::InvalidInput(format!(
            "{command} would produce {produced} entities; the array cap is {MAX_ARRAY_ENTITIES}"
        )))
    }
}

/// Relabel an unsupported transform refusal with the command context (matching the
/// COPY/MIRROR style).
fn array_transform_refusal(command: &str, source: &DbEntity, error: CadError) -> CadError {
    match error {
        CadError::Unsupported(reason) => CadError::Unsupported(format!(
            "{command} cannot transform {}: {reason}",
            geometry_kind(&source.geometry)
        )),
        other => other,
    }
}

/// Report a per-entity display override that the copy path cannot carry (the same
/// honest fallback COPY/MIRROR report), never a silent style change.
fn push_array_style_diagnostic(
    drawing: &DrawingDatabase,
    source: &DbEntity,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if drawing.entity_render_attributes(source.id) != cad_db::EntityRenderAttributes::default() {
        diagnostics.push(Diagnostic {
            object: Some(ObjectId(source.id.0)),
            code: "drawing.array.style".into(),
            message: format!(
                "实体 {} 的按对象颜色/线型/线宽覆盖未复制，副本继承图层样式",
                source.id.0
            ),
        });
    }
}

/// The locale-neutral array summary diagnostic.
fn array_diagnostic(command: &str, copies: usize) -> Diagnostic {
    Diagnostic {
        object: None,
        code: format!("drawing.{}", command.to_ascii_lowercase()),
        message: format!(
            "{command}: {copies} independent copies inserted (sources retained; \
             associative arrays NOT IMPLEMENTED)"
        ),
    }
}

/// The explicit "no copies produced" diagnostic for a single-cell array.
///
/// `rows × columns` / `items` count the source object itself, so a 1×1 / 1-item
/// array leaves the source in place and inserts nothing. That is a real result
/// (`0`), reported explicitly, never a silent empty success.
fn array_no_copies_diagnostic(command: &str) -> Diagnostic {
    Diagnostic {
        object: None,
        code: format!("drawing.{}", command.to_ascii_lowercase()),
        message: format!(
            "{command}: 0 copies produced (the single cell/item is the retained source \
             itself; associative arrays NOT IMPLEMENTED)"
        ),
    }
}
