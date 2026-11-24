//! `DIMROTATED` (AutoCAD 2026): a linear dimension at an arbitrary angle.
//!
//! Split out of `dimension` for the 1000-line code gate. The command, its two
//! documented input forms ([`crate::RotatedLinearDimensionCommand::Points`] and
//! [`crate::RotatedLinearDimensionCommand::Object`]), the text override and the
//! object-origin helpers all live here; the shared synthesis and create funnel are
//! still `super::dimension`'s.
//!
//! Every refusal is named and happens before any write, so it opens no
//! transaction and records no history. Work-only; non-associative.

use super::dimension::MIN_DIMENSION_EXTENT;
use super::*;

// ---------------------------------------------------------------------------
// DIMROTATED (AutoCAD 2026): a linear dimension with an arbitrary angle.
// ---------------------------------------------------------------------------

/// Refuse with a stable `dimrotated.<code>: <detail>` message.
fn dimrotated_error(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("dimrotated.{code}: {detail}"))
}

impl Application {
    /// DIMROTATED: a **rotated** linear dimension (AutoCAD 2026).
    ///
    /// Payload is [`CommandPayload::RotatedLinearDimension`], which carries the two
    /// documented forms:
    ///
    /// * [`crate::RotatedLinearDimensionCommand::Points`] — the three-pick form:
    ///   two extension-line origins plus the dimension-line location.
    /// * [`crate::RotatedLinearDimensionCommand::Object`] — AutoCAD's
    ///   "select object" form: the selected entity supplies both origins (its two
    ///   endpoints for an open LINE/ARC/elliptical ARC/SPLINE/LWPOLYLINE; the two
    ///   extent points along the dimension-line direction for a CIRCLE or a full
    ///   ELLIPSE, which is the declared rule for a closed object), and the location
    ///   stays an explicit pick.
    ///
    /// `rotation` is already in radians (the host converts the session
    /// `dimRotation` degrees exactly once). The definition uses the same
    /// `DimensionKind::Linear { rotation }` subtype DIMLINEAR uses, so the shared
    /// synthesis recomputes the measurement as the projection
    /// `|cos θ · Δx + sin θ · Δy|`, the formatted text and the display. An optional
    /// `text_override` is stored through the same field `DIMEDIT TEXT` writes
    /// (`<>` = the measurement) and is validated like `DIMEDIT` before any write.
    ///
    /// Every refusal is named and happens before any write, so it opens no
    /// transaction and records no history. Work-only; non-associative. The
    /// interactive "Specify angle of dimension line" and "Select object" prompts
    /// are global interaction scope; this command models their payload forms.
    pub(crate) fn dim_rotated(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::RotatedLinearDimension(input) = &command.payload else {
            return Err(dimrotated_error(
                "payload",
                "DIMROTATED needs a RotatedLinearDimension payload (the Points form with \
                 { first, second, location, rotation } or the Object form with \
                 { target, location, rotation })",
            ));
        };
        let (first, second, location, rotation, text_override) = match input.as_ref() {
            crate::RotatedLinearDimensionCommand::Points {
                first,
                second,
                location,
                rotation,
                text_override,
            } => (*first, *second, *location, *rotation, text_override.clone()),
            crate::RotatedLinearDimensionCommand::Object {
                target,
                location,
                rotation,
                text_override,
            } => {
                if !rotation.is_finite() {
                    return Err(dimrotated_error(
                        "rotation",
                        format!("the dimension-line angle must be finite, got {rotation} rad"),
                    ));
                }
                if !is_finite_point(*location) {
                    return Err(dimrotated_error(
                        "non_finite",
                        "the dimension-line location must be finite",
                    ));
                }
                let (first, second) =
                    self.rotated_object_origins(command.document, target, *rotation)?;
                (first, second, *location, *rotation, text_override.clone())
            }
        };
        if !rotation.is_finite() {
            return Err(dimrotated_error(
                "rotation",
                format!("the dimension-line angle must be finite, got {rotation} rad"),
            ));
        }
        if !is_finite_point(first) || !is_finite_point(second) || !is_finite_point(location) {
            return Err(dimrotated_error(
                "non_finite",
                "all three points must be finite",
            ));
        }
        // The same flat `+Z` subset as DIMLINEAR (ADR 0017 D.6): an off-plane
        // triple is refused rather than guessed onto a work plane.
        let z_tol = 1e-9 * (1.0 + first.z.abs() + second.z.abs() + location.z.abs());
        if (second.z - first.z).abs() > z_tol || (location.z - first.z).abs() > z_tol {
            return Err(dimrotated_error(
                "off_plane",
                "the points must lie in one +Z plane; off-plane dimension creation is NOT \
                 IMPLEMENTED",
            ));
        }
        if distance(first, second) <= MIN_DIMENSION_EXTENT {
            return Err(dimrotated_error(
                "origins",
                "the two extension-line origins must be distinct",
            ));
        }
        if let Some(text) = &text_override {
            if text.trim().is_empty() {
                return Err(dimrotated_error(
                    "bad_text",
                    "the dimension text override must be non-empty; omit it to keep the measured \
                     text",
                ));
            }
            let characters = text.chars().count();
            if characters > MAX_TOOL_TEXT_CHARS {
                return Err(dimrotated_error(
                    "bad_text",
                    format!(
                        "the text override allows at most {MAX_TOOL_TEXT_CHARS} characters, got \
                         {characters}"
                    ),
                ));
            }
        }
        let definition = DimensionDefinition {
            kind: DimensionKind::Linear { rotation },
            points: DimensionPoints::Linear {
                first,
                second,
                dimension_line: location,
            },
            // Group 11/53 stay at the zero "derive from the dimension line"
            // sentinel, so the synthesis places the text itself (ADR 0017 A.3).
            text_middle: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            text_rotation: 0.0,
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        };
        let document_id = command.document;
        let (style_name, style) = {
            let document = self
                .workspace
                .documents
                .get(&document_id)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            super::helpers::current_dim_style(document.drawing.as_ref(), session)?
        };
        let dimension =
            DimensionGeometry::synthesized(definition, style_name, style, text_override)?;
        // The chosen angle projects the two distinct origins onto the same point:
        // refuse rather than store a dimension that measures nothing.
        if dimension.measurement() <= MIN_DIMENSION_EXTENT {
            return Err(dimrotated_error(
                "degenerate",
                format!(
                    "the {} degree dimension-line direction measures zero",
                    rotation.to_degrees()
                ),
            ));
        }
        self.commit_new_entity(
            session,
            command,
            SemanticGeometry::Dimension(Box::new(dimension)),
            "create dimrotated",
        )
    }

    /// Resolve the object-selection form's two extension-line origins.
    ///
    /// An open curve (LINE, ARC, elliptical ARC, open SPLINE, open LWPOLYLINE)
    /// supplies its two endpoints exactly. A CIRCLE or a full ELLIPSE has no
    /// endpoints, so the declared rule is the two **extent points along the
    /// dimension-line direction**: `center ± s·(cos θ, sin θ)`, where `s` is the
    /// radius (circle) or the ellipse's radial extent along that direction, which
    /// makes the measured projection exactly the full extent. A closed LWPOLYLINE
    /// has no two-origin model and every other kind (INSERT, HATCH, TEXT, MESH, …)
    /// is refused by name; nothing is written.
    fn rotated_object_origins(
        &self,
        document_id: DocumentId,
        target: &SelectionRef,
        rotation: f64,
    ) -> CadResult<(Point3, Point3)> {
        if target.document != document_id {
            return Err(dimrotated_error(
                "not_addressable",
                format!("entity {} belongs to another document", target.entity.0),
            ));
        }
        if !target.instance.0.is_empty() || target.sub_element.is_some() {
            return Err(dimrotated_error(
                "not_addressable",
                format!(
                    "entity {} is referenced through a block instance or a sub-element",
                    target.entity.0
                ),
            ));
        }
        let document = self
            .workspace
            .documents
            .get(&document_id)
            .ok_or_else(|| dimrotated_error("not_addressable", "document not open"))?;
        let entity = document.drawing.entity(target.entity).ok_or_else(|| {
            dimrotated_error(
                "not_addressable",
                format!("entity {} does not exist", target.entity.0),
            )
        })?;
        let direction = Point3 {
            x: rotation.cos(),
            y: rotation.sin(),
            z: 0.0,
        };
        match &entity.geometry {
            SemanticGeometry::Polyline { points, closed, .. } if !*closed => {
                let (Some(first), Some(last)) = (points.first(), points.last()) else {
                    return Err(dimrotated_error(
                        "object_degenerate",
                        "the selected POLYLINE has no two endpoints",
                    ));
                };
                Ok((*first, *last))
            }
            SemanticGeometry::Circle {
                center,
                normal,
                radius,
            } => circle_extent_origins(*center, *normal, *radius, direction),
            SemanticGeometry::Ellipse {
                center,
                normal,
                major_axis,
                ratio,
                sweep,
                ..
            } => {
                if sweep.abs() < std::f64::consts::TAU - 1e-9 {
                    // An elliptical ARC is open: its stored parameter endpoints are
                    // the origins.
                    return ellipse_curve_origins(&entity.geometry);
                }
                ellipse_extent_origins(*center, *normal, *major_axis, *ratio, direction)
            }
            // A full-turn ARC is a closed circle: same extent rule as a CIRCLE.
            SemanticGeometry::Arc {
                center,
                normal,
                radius,
                sweep,
                ..
            } if sweep.abs() >= std::f64::consts::TAU - 1e-9 => {
                circle_extent_origins(*center, *normal, radius.abs(), direction)
            }
            SemanticGeometry::Line { .. }
            | SemanticGeometry::Arc { .. }
            | SemanticGeometry::Spline { .. } => ellipse_curve_origins(&entity.geometry),
            other => Err(dimrotated_error(
                "object_unsupported",
                format!(
                    "the selected entity is {}; DIMROTATED's object form needs an open LINE, \
                     ARC, elliptical ARC, open SPLINE, open LWPOLYLINE, CIRCLE or full ELLIPSE",
                    class_name(other)
                ),
            )),
        }
    }
}

/// The two parameter endpoints of an open single-curve target (LINE, ARC,
/// elliptical ARC or SPLINE) through the shared TRIM/EXTEND target model.
fn ellipse_curve_origins(geometry: &SemanticGeometry) -> CadResult<(Point3, Point3)> {
    let curve = TargetCurve::new_trim_extend(
        geometry,
        "DIMROTATED",
        "an open LINE, ARC, elliptical ARC, open SPLINE or open LWPOLYLINE",
    )?;
    Ok((curve.point_at(0.0), curve.point_at(1.0)))
}

/// The two extent points of a CIRCLE along `direction` (a flat `+Z` circle only).
fn circle_extent_origins(
    center: Point3,
    normal: Point3,
    radius: f64,
    direction: Point3,
) -> CadResult<(Point3, Point3)> {
    require_xy_plane(normal, "CIRCLE")?;
    if !radius.is_finite() || radius <= MIN_DIMENSION_EXTENT {
        return Err(dimrotated_error(
            "object_degenerate",
            "the selected CIRCLE has a degenerate radius",
        ));
    }
    Ok(extent_pair(center, radius, direction))
}

/// The two extent points of a full ELLIPSE along `direction` (a flat `+Z` ellipse
/// only). The domain frame is the one `TargetCurve` uses: `minor` runs along
/// `cross(normal_unit, major_unit)` with length `|major_axis| · ratio`.
fn ellipse_extent_origins(
    center: Point3,
    normal: Point3,
    major_axis: Point3,
    ratio: f64,
    direction: Point3,
) -> CadResult<(Point3, Point3)> {
    require_xy_plane(normal, "ELLIPSE")?;
    let major_len = cad_geometry::length(major_axis);
    if !major_len.is_finite() || major_len <= MIN_DIMENSION_EXTENT {
        return Err(dimrotated_error(
            "object_degenerate",
            "the selected ELLIPSE has a degenerate major axis",
        ));
    }
    let minor_len = major_len * ratio;
    if !minor_len.is_finite() || minor_len <= MIN_DIMENSION_EXTENT {
        return Err(dimrotated_error(
            "object_degenerate",
            "the selected ELLIPSE has a degenerate minor axis",
        ));
    }
    let u = Point3 {
        x: major_axis.x / major_len,
        y: major_axis.y / major_len,
        z: 0.0,
    };
    // cross(+Z, û) for the domain's positive normal.
    let v = Point3 {
        x: -u.y,
        y: u.x,
        z: 0.0,
    };
    let along_u = cad_geometry::dot(direction, u) / major_len;
    let along_v = cad_geometry::dot(direction, v) / minor_len;
    let scale_sq = along_u * along_u + along_v * along_v;
    if !scale_sq.is_finite() || scale_sq <= 0.0 {
        return Err(dimrotated_error(
            "object_degenerate",
            "the selected ELLIPSE has no extent along the dimension-line direction",
        ));
    }
    Ok(extent_pair(center, 1.0 / scale_sq.sqrt(), direction))
}

/// `center ± extent · direction` on the centre's own z plane.
fn extent_pair(center: Point3, extent: f64, direction: Point3) -> (Point3, Point3) {
    (
        Point3 {
            x: center.x + extent * direction.x,
            y: center.y + extent * direction.y,
            z: center.z,
        },
        Point3 {
            x: center.x - extent * direction.x,
            y: center.y - extent * direction.y,
            z: center.z,
        },
    )
}

/// Refuse a conic whose plane is not parallel to world XY: the rotated linear
/// dimension is flat (`+Z`), so a tilted conic has no two extent points on it.
fn require_xy_plane(normal: Point3, kind: &str) -> CadResult<()> {
    let z_tol = 1e-9;
    if normal.x.abs() > z_tol || normal.y.abs() > z_tol || normal.z.abs() <= 0.5 {
        return Err(dimrotated_error(
            "object_off_plane",
            format!(
                "the selected {kind} is not parallel to world XY; a rotated linear dimension \
                 is flat (+Z), so a tilted {kind} has no two extent points on that plane"
            ),
        ));
    }
    Ok(())
}
