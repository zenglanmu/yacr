//! Typed Measurements dimensions for LINE and RECTANG.
//!
//! The pure capture never sees session parameters; this module owns the small
//! composition step that turns a validated `ToolParameter` into the endpoint or
//! second corner the confirm will commit. Split out of `session_state.rs` to keep
//! that module inside the code-size gate.
use super::*;

impl SessionState {
    /// The endpoint a typed LINE length/direction selects, or `None` when the
    /// capture keeps the cursor-defined one.
    ///
    /// `LineLength` is the unset sentinel at `0`; `LineAngle` is only consulted
    /// with a positive length, and `0` there means "keep the cursor direction"
    /// (a 0° direction cannot be forced through this channel yet).
    pub(super) fn typed_line_end(&self, start: Point3, cursor: Option<Point3>) -> Option<Point3> {
        let length = self.tool_parameter(ToolParameter::LineLength).number()?;
        if !length.is_finite() || length <= 0.0 {
            return None;
        }
        let angle = self
            .tool_parameter(ToolParameter::LineAngle)
            .number()
            .unwrap_or(0.0);
        let (dx, dy) = if angle.is_finite() && angle != 0.0 {
            let radians = angle.to_radians();
            (radians.cos(), radians.sin())
        } else {
            let end = cursor?;
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let distance = dx.hypot(dy);
            if !distance.is_finite() || distance <= f64::EPSILON {
                return None;
            }
            (dx / distance, dy / distance)
        };
        Some(Point3 {
            x: start.x + dx * length,
            y: start.y + dy * length,
            z: start.z,
        })
    }

    /// The second RECTANG corner a typed width/height selects, or `None` when
    /// neither extent is set (the capture keeps the cursor's corner).
    ///
    /// A typed extent keeps the cursor's **sign** on that axis, so dragging left
    /// or up still builds the rectangle in that quadrant; the other axis keeps the
    /// cursor's value verbatim.
    pub(super) fn typed_rect_corner(&self, corner1: Point3, cursor: Point3) -> Option<Point3> {
        let width = self
            .tool_parameter(ToolParameter::RectWidth)
            .number()
            .unwrap_or(0.0);
        let height = self
            .tool_parameter(ToolParameter::RectHeight)
            .number()
            .unwrap_or(0.0);
        let width = width.is_finite() && width > 0.0;
        let height = height.is_finite() && height > 0.0;
        if !width && !height {
            return None;
        }
        let x = if width {
            let sign = if cursor.x < corner1.x { -1.0 } else { 1.0 };
            corner1.x
                + sign
                    * self
                        .tool_parameter(ToolParameter::RectWidth)
                        .number()
                        .unwrap_or(0.0)
        } else {
            cursor.x
        };
        let y = if height {
            let sign = if cursor.y < corner1.y { -1.0 } else { 1.0 };
            corner1.y
                + sign
                    * self
                        .tool_parameter(ToolParameter::RectHeight)
                        .number()
                        .unwrap_or(0.0)
        } else {
            cursor.y
        };
        Some(Point3 { x, y, z: cursor.z })
    }

    /// The edge point a typed CIRCLE radius selects, or `None` when the radius is
    /// unset (the capture keeps the cursor-defined edge).
    ///
    /// The direction is the cursor's, so the typed radius owns only the distance;
    /// a zero-length direction has no honest edge and leaves the capture raw (the
    /// command then refuses the degenerate circle).
    pub(super) fn typed_circle_edge(
        &self,
        center: Point3,
        cursor: Option<Point3>,
    ) -> Option<Point3> {
        let radius = self.tool_parameter(ToolParameter::CircleRadius).number()?;
        if !radius.is_finite() || radius <= 0.0 {
            return None;
        }
        let edge = cursor?;
        let dx = edge.x - center.x;
        let dy = edge.y - center.y;
        let distance = dx.hypot(dy);
        if !distance.is_finite() || distance <= f64::EPSILON {
            return None;
        }
        Some(Point3 {
            x: center.x + dx / distance * radius,
            y: center.y + dy / distance * radius,
            z: center.z,
        })
    }

    /// The preview cursor a typed ROTATE angle selects, or `None` when the angle
    /// is unset (the capture keeps the cursor-defined angle).
    ///
    /// The radius stays the cursor's distance from the base, so only the
    /// direction is replaced. A cursor on the base has no radius to rotate.
    pub(super) fn typed_rotate_cursor(
        &self,
        base: Point3,
        cursor: Option<Point3>,
    ) -> Option<Point3> {
        let degrees = self.tool_parameter(ToolParameter::RotateAngle).number()?;
        if !degrees.is_finite() || degrees == 0.0 {
            return None;
        }
        let cursor = cursor?;
        let dx = cursor.x - base.x;
        let dy = cursor.y - base.y;
        let radius = dx.hypot(dy);
        if !radius.is_finite() || radius <= f64::EPSILON {
            return None;
        }
        let radians = degrees.to_radians();
        Some(Point3 {
            x: base.x + radius * radians.cos(),
            y: base.y + radius * radians.sin(),
            z: base.z,
        })
    }

    /// The displacement a typed MOVE distance selects, or `None` when the distance
    /// is unset.
    ///
    /// The cursor's direction is kept; only the magnitude changes. A zero
    /// displacement has no direction to scale, so the capture keeps it (the
    /// command then refuses the no-op).
    pub(super) fn typed_move_delta(&self, delta: Point3) -> Option<Point3> {
        let distance = self.tool_parameter(ToolParameter::MoveDistance).number()?;
        if !distance.is_finite() || distance <= 0.0 {
            return None;
        }
        let magnitude = delta.x.hypot(delta.y).hypot(delta.z);
        if !magnitude.is_finite() || magnitude <= f64::EPSILON {
            return None;
        }
        let scale = distance / magnitude;
        Some(Point3 {
            x: delta.x * scale,
            y: delta.y * scale,
            z: delta.z * scale,
        })
    }
    /// Attach the session side count to both POLYGON previews and constrain the
    /// pending point entry (grid/ortho) so the rubber band matches the committed
    /// point; a pending entity pick stays raw.
    pub(super) fn annotate_draw_preview(&self, mut preview: DrawPreview) -> DrawPreview {
        preview.polygon_sides = matches!(
            preview.kind,
            DrawToolKind::Polygon | DrawToolKind::PolygonCircumscribed
        )
        .then_some(self.polygon_sides());
        let pending_pick = matches!(
            &self.tool,
            ToolState::Drawing(capture) if capture.tool.requires_entity_pick()
        );
        if !pending_pick {
            preview.cursor =
                self.constrained_cursor(preview.points.last().copied(), preview.cursor);
        }
        // A typed LINE length/direction owns the pending endpoint, so the rubber
        // band shows exactly the segment the confirm will create. The pure tool
        // never sees the session parameters; this is the one place that composes
        // them.
        if preview.kind == DrawToolKind::Line && preview.points.len() == 1 {
            if let Some(end) = self.typed_line_end(preview.points[0], preview.cursor) {
                preview.cursor = Some(end);
            }
        }
        if preview.kind == DrawToolKind::Move && preview.points.len() == 1 {
            if let Some(cursor) = preview.cursor {
                let delta = Point3 {
                    x: cursor.x - preview.points[0].x,
                    y: cursor.y - preview.points[0].y,
                    z: cursor.z - preview.points[0].z,
                };
                if let Some(typed) = self.typed_move_delta(delta) {
                    preview.cursor = Some(Point3 {
                        x: preview.points[0].x + typed.x,
                        y: preview.points[0].y + typed.y,
                        z: preview.points[0].z + typed.z,
                    });
                }
            }
        }
        if preview.kind == DrawToolKind::Scale && preview.points.len() >= 2 {
            if let Some(target) = self.typed_scale_target(preview.points[0], preview.points[1]) {
                preview.cursor = Some(target);
            }
        }
        if preview.kind == DrawToolKind::Rotate && preview.points.len() == 1 {
            if let Some(cursor) = self.typed_rotate_cursor(preview.points[0], preview.cursor) {
                preview.cursor = Some(cursor);
            }
        }
        if preview.kind == DrawToolKind::Circle && preview.points.len() == 1 {
            if let Some(edge) = self.typed_circle_edge(preview.points[0], preview.cursor) {
                preview.cursor = Some(edge);
            }
        }
        if preview.kind == DrawToolKind::Rectang && preview.points.len() == 1 {
            // A typed extent owns the matching axis; the other axis keeps the
            // cursor's own value. Before the pointer moves there is no second
            // corner to complete, so the raw cursor (or its absence) stays.
            if let Some(cursor) = preview.cursor {
                if let Some(corner) = self.typed_rect_corner(preview.points[0], cursor) {
                    preview.cursor = Some(corner);
                }
            }
        }
        preview
    }

    pub fn draw_intent(&self) -> CadResult<DrawIntent> {
        let intent = match &self.tool {
            ToolState::Drawing(capture) => capture.tool.intent()?,
            _ => return Err(CadError::InvalidInput("no drawing tool is active".into())),
        };
        // The typed dimensions are session state, so the pure capture hands back
        // the cursor-defined geometry and this layer replaces it. A positive LINE
        // length with neither an angle nor a cursor direction is refused by name
        // instead of silently solidifying a zero-length line.
        match intent {
            DrawIntent::Line { start, end } => {
                let length = self
                    .tool_parameter(ToolParameter::LineLength)
                    .number()
                    .unwrap_or(0.0);
                if !length.is_finite() || length <= 0.0 {
                    return Ok(DrawIntent::Line { start, end });
                }
                let Some(typed) = self.typed_line_end(start, Some(end)) else {
                    return Err(CadError::InvalidInput(
                        "line.direction: a typed LINE length needs a non-zero direction (move the cursor or set lineAngle)"
                            .into(),
                    ));
                };
                Ok(DrawIntent::Line { start, end: typed })
            }
            DrawIntent::Scale { base, factor } => {
                let typed = self
                    .tool_parameter(ToolParameter::ScaleFactor)
                    .number()
                    .unwrap_or(0.0);
                if !typed.is_finite() || typed <= 0.0 {
                    return Ok(DrawIntent::Scale { base, factor });
                }
                Ok(DrawIntent::Scale {
                    base,
                    factor: typed,
                })
            }
            DrawIntent::Move { delta } => {
                let Some(typed) = self.typed_move_delta(delta) else {
                    return Ok(DrawIntent::Move { delta });
                };
                Ok(DrawIntent::Move { delta: typed })
            }
            DrawIntent::Rotate { base, angle } => {
                let degrees = self
                    .tool_parameter(ToolParameter::RotateAngle)
                    .number()
                    .unwrap_or(0.0);
                if !degrees.is_finite() || degrees == 0.0 {
                    return Ok(DrawIntent::Rotate { base, angle });
                }
                Ok(DrawIntent::Rotate {
                    base,
                    angle: degrees.to_radians(),
                })
            }
            DrawIntent::Circle { center, edge } => {
                let Some(typed) = self.typed_circle_edge(center, Some(edge)) else {
                    return Ok(DrawIntent::Circle { center, edge });
                };
                Ok(DrawIntent::Circle {
                    center,
                    edge: typed,
                })
            }
            DrawIntent::Rect { corner1, corner2 } => {
                let Some(typed) = self.typed_rect_corner(corner1, corner2) else {
                    return Ok(DrawIntent::Rect { corner1, corner2 });
                };
                Ok(DrawIntent::Rect {
                    corner1,
                    corner2: typed,
                })
            }
            other => Ok(other),
        }
    }

    /// The target point a typed SCALE factor selects, or `None` when the factor
    /// is unset.
    ///
    /// The capture's second point is the reference vector from the base; the typed
    /// factor scales that vector. A zero reference has no direction to scale.
    pub(super) fn typed_scale_target(&self, base: Point3, reference: Point3) -> Option<Point3> {
        let factor = self.tool_parameter(ToolParameter::ScaleFactor).number()?;
        if !factor.is_finite() || factor <= 0.0 {
            return None;
        }
        let dx = reference.x - base.x;
        let dy = reference.y - base.y;
        let dz = reference.z - base.z;
        if dx.hypot(dy).hypot(dz) <= f64::EPSILON {
            return None;
        }
        Some(Point3 {
            x: base.x + dx * factor,
            y: base.y + dy * factor,
            z: base.z + dz * factor,
        })
    }
}
