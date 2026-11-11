//! ARRAYRECT / ARRAYPOLAR / ARRAYPATH edit commands (ADR 0016 decision D).
//!
//! All three commands insert **independent copies** of the current selection using
//! the existing N-copies-in-one-transaction shape (`copy_entities`/`mirror_selection`):
//! every copy is a fresh entity, the sources are **retained**, and the whole array
//! is one transaction with one undo step. There is **no** second insertion path and
//! no associativity (AutoCAD's associative array model is NOT IMPLEMENTED — the
//! project has no associative object/grip mechanism).
//!
//! ARRAYPATH v1 is the 2D **path** array: the first payload ref is the path and
//! every remaining ref is an item, copied to equal arc-length positions along
//! that path (see [`Application::array_path`]). `count` is the number of
//! **copies**; the N source objects stay, and position 0 can coincide with a
//! retained source whose anchor equals the path start. A periodic path (CIRCLE,
//! full-turn ARC/ELLIPSE, closed LWPOLYLINE) uses `i / count` so the seam is
//! never duplicated.
//!
//! The placement maths are our **declared** rules (AutoCAD's exact base-point
//! placement is unverified); they are documented here and pinned by the contract
//! tests. The one rule that deliberately follows AutoCAD's documented semantics is
//! ARRAYPOLAR's fill/items relationship (`fill / (items − 1)` over a partial fill,
//! `fill / items` over a full turn — see [`Application::array_polar`]).
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

/// A full turn in the prompt unit (degrees). `|fillAngle| == 360` is ARRAYPOLAR's
/// full-circle default, and it is the one fill that divides by `items` instead of
/// `items − 1` (see [`Application::array_polar`]).
const FULL_TURN_DEGREES: f64 = 360.0;

impl Application {
    /// ARRAYRECT: insert independent copies of the selection on a rectangular grid.
    ///
    /// Our declared geometry rule (AutoCAD's exact rule is unverified): the base
    /// point is the selection's **bounding-box lower-left**, and cell `(row,
    /// column)` translates the source by `(column·columnSpacing, row·rowSpacing)`
    /// — spacing is the **distance between base points**, and a positive spacing
    /// grows the array **up and to the right**. (The offsets are measured between
    /// base points, so the numeric result is independent of where the declared
    /// base sits.)
    ///
    /// **Signed spacing** (`-ARRAY`'s negative spacing): a negative `rowSpacing`
    /// (resp. `columnSpacing`) arrays **downward** (respectively to the left),
    /// because the cell offset *is* `row · rowSpacing`. Nothing is mirrored and
    /// the source's own cell stays `(0, 0)`, so the sign only selects the
    /// direction of growth. Zero spacing is legal (the cells coincide) and is
    /// reported by the ordinary copy count, never hidden.
    ///
    /// **Copy count** (ADR 0016 decision D): `rows × columns` is the total
    /// **including the source object**, which stays where it is and is not
    /// duplicated, so the array inserts `rows × columns − 1` copies. The source's
    /// own cell `(0, 0)` is skipped. A 1×1 "array" therefore produces **0 copies**;
    /// that is an explicit diagnostic, never a silent success.
    ///
    /// **NOT IMPLEMENTED — `Levels` (the Z direction) and the rotated grid
    /// (`ANGLE`)**: the session tool-parameter channel this build exposes carries
    /// neither an array level count nor a grid angle (the frozen
    /// `ToolParameter` vocabulary has `ArrayRows`/`ArrayColumns`/spacings only),
    /// so an ARRAYRECT is always **one level** on an **axis-aligned** grid. There
    /// is no implicit rotation and no invented second store: a request that would
    /// need either option cannot be expressed, so nothing is silently approximated.
    ///
    /// Refusals (nothing written): non-integer or `< 1` rows/columns; non-finite
    /// spacing; an empty selection; source geometry that cannot be translated (the
    /// existing `Unsupported` surface); and a total produced copy count above
    /// [`MAX_ARRAY_ENTITIES`].
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
                point_report: None,
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
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        diagnostics.push(array_diagnostic("ARRAYRECT", copies.len()));
        Ok(CommandOutcome {
            objects: copies.iter().map(|(id, _)| ObjectId(id.0)).collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// ARRAYPOLAR: insert independent copies of the selection around `center`.
    ///
    /// **Angular placement** (AutoCAD's documented `Fill`/`Items` relationship):
    /// the fill angle spans the first and the last item, so the step between
    /// consecutive items is `fillAngle / (items − 1)` and item `i` sits at
    /// `i · step` counter-clockwise from the base point's direction (the base
    /// point is the selection's **bounding-box lower-left**). A **full turn**
    /// (`|fillAngle| == 360`) is the one exception: it closes on itself, so the
    /// step is `fillAngle / items` and the last item never duplicates the
    /// source's own position. This is pinned numerically by the contract tests
    /// (`180° / 3 items` ⇒ `0°/90°/180°`, `360° / 4 items` ⇒ `0°/90°/180°/270°`).
    /// With `rotateItems = true` each copy is **rotated about the centre** by its
    /// own angle; with `false` each copy is **translated only** to its angular
    /// position, its own orientation unchanged. The step rule is the same in both
    /// cases: `rotateItems` decides orientation only, never position.
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
        // `fillAngle / (items - 1)` over a partial fill, `fillAngle / items` over a
        // full turn (see the method doc). `items` is validated `>= 1`; a 1-item
        // array has already returned above when it produces no copies, so the
        // divisor can only be 1 or more here.
        let full_turn = (fill_angle.abs() - FULL_TURN_DEGREES).abs() <= 1e-9;
        let step_divisor = if full_turn {
            items
        } else {
            (items - 1.0).max(1.0)
        };

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
                point_report: None,
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
            let delta = item as f64 * fill_radians / step_divisor;
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
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        diagnostics.push(array_diagnostic("ARRAYPOLAR", copies.len()));
        Ok(CommandOutcome {
            objects: copies.iter().map(|(id, _)| ObjectId(id.0)).collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
        })
    }

    /// ARRAYPATH: insert independent copies of the selected items along the
    /// selected path.
    ///
    /// **Input rule (v1)**: the **first `refs` entry is the path**; every
    /// remaining entry is an **item**. The refs travel in the payload (the typed
    /// path fills them from the session selection at argument-dispatch time), so
    /// the handler never re-reads a mutable selection. The path must be one of
    /// the kinds the shared equal-distance walker supports (`LINE`, `ARC`,
    /// `CIRCLE`, `SPLINE`, `ELLIPSE`, open/closed `LWPOLYLINE`); the source items
    /// are retained and the path itself is never duplicated. Every ref must be a
    /// directly addressable model-or-paper entity (no block-instance member, no
    /// sub-element) and all of them must share the path's [`SpaceId`].
    ///
    /// **Placement (declared rule)**: each item's own anchor is its block base
    /// point mapped to world (`INSERT`) or its bounding-box XY minimum (every
    /// other kind), carried at the path start's Z. The group anchor is the
    /// componentwise minimum of those per-item anchors; every position translates
    /// the whole group by `position − group anchor`.
    ///
    /// **Positions**: `count` is the number of **copies** (the N source objects
    /// stay; position 0 can coincide with a retained source whose anchor equals
    /// the path start). An **open** path places the copies at `i / (count - 1)`
    /// of its arc length (**both ends included**); a **periodic** path (CIRCLE,
    /// full-turn ARC, full ELLIPSE, closed LWPOLYLINE) places them at
    /// `i / count`, so `point_at(1.0) == point_at(0.0)` never produces a seam
    /// duplicate. `count == 1` places the single copy at the path start.
    ///
    /// **`ALIGN` (declared approximation)**: each copy keeps the group
    /// translation and is rotated **in place about its own translated anchor** by
    /// `tangent_i − tangent_0`, so a spread selection never swings around the
    /// path point. A position with no in-plane (XY) tangent (a purely vertical
    /// path) refuses `arraypath.no_plane_tangent`; without `ALIGN` the
    /// orientation is unchanged and no tangent is needed.
    ///
    /// One transaction / one undo step; all inputs are validated before any write,
    /// so a refusal changes nothing. Refusals: fewer than two payload refs (a
    /// path needs at least one item), a block-instance member or sub-element
    /// (`arraypath.block_member`), a path/item space mismatch
    /// (`arraypath.space_mismatch`), an unsupported or zero-length path,
    /// `count == 0`, an uncomputable item anchor, an untransformable item and a
    /// total copy count above [`MAX_ARRAY_ENTITIES`]. Associativity is **NOT
    /// IMPLEMENTED**.
    pub(crate) fn array_path(
        &mut self,
        _session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ArrayPath { refs, count, align } = &command.payload else {
            return Err(CadError::InvalidInput(
                "arraypath.needs_argument: ARRAYPATH needs a typed position count, e.g. \
                 `ARRAYPATH 4 [ALIGN]`; nothing was changed"
                    .into(),
            ));
        };
        let refs = refs.clone();
        let count = *count;
        let align = *align;
        if count == 0 {
            return Err(CadError::InvalidInput(
                "arraypath.invalid_count: ARRAYPATH needs a copy count >= 1".into(),
            ));
        }
        if refs.len() < 2 {
            return Err(CadError::InvalidInput(
                "arraypath.selection: ARRAYPATH needs a path and at least one item (select two or \
                 more objects, path first)"
                    .into(),
            ));
        }
        // Directly addressable entities only: a block-instance member or a
        // sub-element is never mistaken for a whole selectable path/item. The
        // check runs before the drawing is read (matching DIVIDE's guard).
        for reference in &refs {
            if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
                return Err(CadError::InvalidInput(format!(
                    "arraypath.block_member: ARRAYPATH target {} is a block-instance member or \
                     sub-element; only directly addressable entities can be arrayed",
                    reference.entity.0
                )));
            }
        }

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
            &refs,
            "arraypath",
            "ARRAYPATH",
            "array",
            false,
        )?;
        if sources.len() < 2 {
            return Err(CadError::InvalidInput(
                "arraypath.selection: ARRAYPATH needs a path and at least one item (select two or \
                 more objects, path first)"
                    .into(),
            ));
        }
        let (path, items) = sources.split_first().expect("checked at least two sources");

        // The path and every item must live in the same space: a path in model
        // space cannot place copies of a paper-space or block-owned item.
        for item in items {
            if item.space != path.space {
                return Err(CadError::InvalidInput(format!(
                    "arraypath.space_mismatch: the path (entity {}) and item (entity {}) are in \
                     different spaces",
                    path.id.0, item.id.0
                )));
            }
        }

        // Every placement of every item is a fresh copy (the path itself is not
        // duplicated), so the produced total is `count * items`.
        let produced = count as f64 * items.len() as f64;
        check_array_cap(produced, "ARRAYPATH")?;

        let positions = super::divide::path_distribution(&path.geometry, count)?;
        let anchors = array_path_anchor(drawing, items, positions[0].0)?;
        let shared_anchor = anchors.iter().fold(anchors[0], |group, anchor| Point3 {
            x: group.x.min(anchor.x),
            y: group.y.min(anchor.y),
            z: group.z,
        });
        let tangent_0 = positions[0].1;
        let missing_tangent = if align {
            positions.iter().position(|(_, tangent)| tangent.is_none())
        } else {
            None
        };
        if let Some(index) = missing_tangent {
            return Err(array_path_no_plane_tangent(path.id, index));
        }

        // Refuse the whole command if any item geometry cannot take the copy
        // transform (a proper rotation for ALIGN, a translation otherwise).
        let probe = if align {
            rotation_transform(shared_anchor, 0.0)
        } else {
            Transform3::translation(Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            })
        };
        for item in items {
            transform_geometry(&item.geometry, &probe)
                .map_err(|error| array_transform_refusal("ARRAYPATH", item, error))?;
            push_array_style_diagnostic(drawing, item, &mut diagnostics);
        }

        let base_order = next_draw_order(drawing);
        let mut copies: Vec<(EntityId, DbEntity)> = Vec::with_capacity(produced as usize);
        let mut index = 0usize;
        for (point, tangent) in &positions {
            let offset = Point3 {
                x: point.x - shared_anchor.x,
                y: point.y - shared_anchor.y,
                z: point.z - shared_anchor.z,
            };
            let group_translation = Transform3::translation(offset);
            for (item, anchor) in items.iter().zip(&anchors) {
                let transform = if align {
                    // Keep the group translation, then rotate in place about the
                    // item's own translated anchor:
                    // `T(delta) · T(anchor_i) · R(Δ) · T(−anchor_i)`.
                    let delta = tangent.expect("checked for every position")
                        - tangent_0.expect("checked for position 0");
                    group_translation.matrix_mul(&rotation_transform(*anchor, delta))
                } else {
                    group_translation
                };
                copies.push(array_copy(
                    drawing,
                    item,
                    &transform,
                    base_order + index as i64,
                )?);
                index += 1;
            }
        }

        let label = "array path";
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
                block_patches: Vec::new(),
                layer_patches: Vec::new(),
                layout_patches: Vec::new(),
                merge_key: None,
            },
        )?;

        diagnostics.push(array_path_diagnostic(
            count,
            items.len(),
            path.id,
            class_name(&path.geometry),
            align,
        ));
        Ok(CommandOutcome {
            objects: copies.iter().map(|(id, _)| ObjectId(id.0)).collect(),
            changes: Some(Arc::new(change_set)),
            diagnostics,
            measurement: None,
            point_report: None,
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

/// The row/column spacing of a session parameter: finite (either sign).
///
/// The channel accepts any finite spacing; this command accepts the sign too,
/// because the cell offset *is* `row · rowSpacing` / `column · columnSpacing`, so
/// a negative spacing arrays in the negative direction (down / left) without
/// mirroring or a second code path. Only a non-finite value is refused.
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

/// The per-item ARRAYPATH anchors, carried at the **path start's Z** so a 2D
/// path array stays on the path's plane.
///
/// * `INSERT` — the block's stored **base point** mapped to world by the
///   reference transform (`transform.apply_point(base)`), i.e. the same base point
///   the importer and `INSERT` use. A block with no stored base point is an
///   explicit refusal, never an invented zero.
/// * every other kind — the item's `BoundsAccumulator` XY minimum
///   ([`BoundsAccumulator::finish`] is the single source of truth; there is no
///   duplicated per-kind point match). Its circular/elliptical bound is
///   conservative (`center ± radius·‖transform‖`), so the anchor is a declared
///   bounding-box corner rather than a tight one. An item with no computable box
///   is an explicit refusal, never a silently dropped item.
fn array_path_anchor(
    drawing: &DrawingDatabase,
    items: &[DbEntity],
    path_start: Point3,
) -> CadResult<Vec<Point3>> {
    let mut anchors = Vec::with_capacity(items.len());
    for item in items {
        let anchor = match &item.geometry {
            SemanticGeometry::Insert { block, transform } => {
                let base = drawing.block_base_point(*block).ok_or_else(|| {
                    CadError::Unsupported(format!(
                        "ARRAYPATH cannot determine the insertion base point of block {:?} \
                         referenced by entity {}",
                        block, item.id.0
                    ))
                })?;
                transform.apply_point(base)
            }
            other => {
                let mut bounds = BoundsAccumulator::new();
                bounds.add_geometry(other);
                match bounds.finish() {
                    Some((min, _max)) => min,
                    None => {
                        return Err(CadError::Unsupported(format!(
                            "ARRAYPATH cannot determine a placement anchor for {} (entity {})",
                            class_name(other),
                            item.id.0
                        )))
                    }
                }
            }
        };
        if !is_finite_point(anchor) {
            return Err(CadError::InvalidInput(format!(
                "arraypath.anchor: ARRAYPATH cannot determine a finite placement anchor for \
                 entity {}",
                item.id.0
            )));
        }
        anchors.push(Point3 {
            x: anchor.x,
            y: anchor.y,
            z: path_start.z,
        });
    }
    if anchors.is_empty() {
        return Err(CadError::InvalidInput(
            "arraypath.anchor: ARRAYPATH needs at least one item".into(),
        ));
    }
    Ok(anchors)
}

/// The ARRAYPATH `ALIGN` refusal: the path has no in-plane (XY) tangent at
/// `position`, so orienting a copy there would invent an angle.
fn array_path_no_plane_tangent(path_id: EntityId, position: usize) -> CadError {
    CadError::InvalidInput(format!(
        "arraypath.no_plane_tangent: ARRAYPATH ALIGN needs an in-plane (XY) tangent at every \
         position; position {position} on path entity {} has none (a purely vertical path \
         segment)",
        path_id.0
    ))
}

/// The ARRAYPATH summary diagnostic (`arraypath.created`), naming the copy count,
/// the retained source count, the path entity/kind and whether `ALIGN` was
/// requested.
fn array_path_diagnostic(
    count: usize,
    items: usize,
    path_id: EntityId,
    path_kind: &str,
    align: bool,
) -> Diagnostic {
    Diagnostic {
        object: None,
        code: "arraypath.created".into(),
        message: format!(
            "ARRAYPATH: {count} copies; {items} sources retained; path {} ({path_kind}); \
             align={align}",
            path_id.0
        ),
    }
}
