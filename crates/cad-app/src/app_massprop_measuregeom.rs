//! `MEASUREGEOM`: the command-layer half of the AutoCAD 2026 inquiry router.
//!
//! Split out of `app_massprop.rs` (code-size ceiling) with the bodies unchanged:
//! the routing (`MEASUREGEOM`'s option dispatch), the `RADIUS` read-only report
//! and its helpers. Both are inherent `Application` methods, so the dispatch arm
//! in `application::execute` is untouched.
//!
//! The option set is complete **for the geometry this build models**:
//!
//! * `DISTANCE`, `ANGLE`, `AREA` and `POLYLINE` are turned into the existing
//!   `BeginMeasurement` requests by `cad-ui-core` and never reach this layer; a
//!   direct command-layer call is refused by name (`measuregeom.invalid_payload`)
//!   instead of starting a tool from the wrong layer;
//! * `RADIUS` is a real read-only report of exactly one selected `ARC`/`CIRCLE`;
//! * `VOLUME` is the stable `measuregeom.volume_unsupported` refusal: there is no
//!   3D-solid/volume model in this build, so no number is invented.

use super::*;

impl Application {
    /// `MEASUREGEOM`: route the option that reached the command layer.
    ///
    /// The four tool options (`DISTANCE`/`ANGLE`/`AREA`/`POLYLINE`) never travel
    /// this far: the session turns them into the existing `BeginMeasurement`
    /// requests. `RADIUS` is the read-only report and `VOLUME` the explicit
    /// not-implemented refusal. Full contract:
    /// [`crate::command_id::measuregeom`].
    pub(crate) fn measure_geom(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match command.payload {
            CommandPayload::MeasureGeom(MeasureGeomOption::Radius) => {
                self.measure_geom_radius(session, command)
            }
            CommandPayload::MeasureGeom(MeasureGeomOption::Volume) => Err(CadError::Unsupported(
                "measuregeom.volume_unsupported: MEASUREGEOM VOLUME is not implemented (this build \
                 has no 3D-solid or volume model); no value is reported"
                    .into(),
            )),
            CommandPayload::MeasureGeom(
                option @ (MeasureGeomOption::Distance
                | MeasureGeomOption::Angle
                | MeasureGeomOption::Area
                | MeasureGeomOption::Polyline),
            ) => Err(CadError::InvalidInput(format!(
                "measuregeom.invalid_payload: {} starts the corresponding measurement tool in the \
                 session and has no command-layer report",
                measure_geom_option_name(option)
            ))),
            CommandPayload::None => Err(CadError::InvalidInput(
                "measuregeom.needs_option: MEASUREGEOM needs one of DISTANCE, ANGLE, AREA, \
                 POLYLINE, RADIUS or VOLUME"
                    .into(),
            )),
            _ => Err(CadError::InvalidInput(
                "measuregeom.invalid_payload: MEASUREGEOM takes its option as a \
                 CommandPayload::MeasureGeom value".into(),
            )),
        }
    }

    /// `MEASUREGEOM RADIUS`: report the one selected ARC/CIRCLE's radius family.
    ///
    /// The report is the radius family the modelled geometry really carries: a
    /// circle adds its area, and an arc adds its exact arc length, included angle
    /// and chord. Nothing else is invented — an arc has no area, so no area is
    /// printed for it, and `MEASUREGEOM VOLUME` stays the named refusal.
    fn measure_geom_radius(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let references = session.selection.refs();
        match references {
            [] => {
                return Err(CadError::InvalidInput(
                    "measuregeom.radius.selection_empty: MEASUREGEOM RADIUS needs exactly one \
                     selected ARC or CIRCLE"
                        .into(),
                ));
            }
            [_] => {}
            many => {
                return Err(CadError::InvalidInput(format!(
                    "measuregeom.radius.selection_multiple: MEASUREGEOM RADIUS measures one \
                     ARC/CIRCLE, got {} selected; nothing was measured",
                    many.len()
                )));
            }
        }
        let reference = &references[0];
        if reference.document != command.document {
            return Err(CadError::InvalidInput(format!(
                "measuregeom.radius.target_missing: the selected ref for entity {} belongs to \
                 another document",
                reference.entity.0
            )));
        }
        if !reference.instance.0.is_empty() || reference.sub_element.is_some() {
            return Err(CadError::Unsupported(
                "measuregeom.radius.not_addressable: block-instance members and sub-elements have \
                 no addressable entity identity"
                    .into(),
            ));
        }
        let entity = {
            let document = self
                .workspace
                .documents
                .get(&command.document)
                .ok_or_else(|| CadError::InvalidInput("document not open".into()))?;
            document
                .drawing
                .entity(reference.entity)
                .cloned()
                .ok_or_else(|| {
                    CadError::InvalidInput(format!(
                        "measuregeom.radius.target_missing: entity {} does not exist in this \
                         drawing",
                        reference.entity.0
                    ))
                })?
        };
        let (arc, center, radius) = match &entity.geometry {
            SemanticGeometry::Circle {
                center,
                normal: _,
                radius,
            } => {
                validate_radius_entity(&entity, *center, *radius)?;
                (None, *center, *radius)
            }
            SemanticGeometry::Arc {
                center,
                normal: _,
                radius,
                start: _,
                sweep,
            } => {
                validate_radius_entity(&entity, *center, *radius)?;
                if !sweep.is_finite() {
                    return Err(CadError::InvalidInput(format!(
                        "measuregeom.radius.degenerate: entity {} has a non-finite sweep",
                        entity.id.0
                    )));
                }
                // The arc's own modelled start/sweep give the chord exactly:
                // |chord| = 2r·|sin(sweep/2)|.
                let half = sweep.abs() * 0.5;
                let chord = 2.0 * radius * half.sin().abs();
                if !chord.is_finite() {
                    return Err(CadError::InvalidInput(format!(
                        "measuregeom.radius.degenerate: entity {}'s chord is not finite",
                        entity.id.0
                    )));
                }
                (
                    Some(ArcProfile {
                        arc_length: radius * sweep.abs(),
                        sweep: *sweep,
                        chord,
                    }),
                    *center,
                    *radius,
                )
            }
            other => {
                return Err(CadError::Unsupported(format!(
                    "measuregeom.radius.not_arc_circle: MEASUREGEOM RADIUS measures one ARC or \
                     CIRCLE; entity {} is {}",
                    entity.id.0,
                    geometry_kind(other)
                )));
            }
        };
        let diameter = 2.0 * radius;
        let circumference = std::f64::consts::TAU * radius;
        // The circle's own area is exact (pi r^2); an arc encloses no area, so the
        // arc report carries its length/angle/chord instead of a fabricated area.
        let extra = match arc {
            Some(profile) => format!(
                " arc_length={:.6} sweep_deg={:.6} chord={:.6}",
                profile.arc_length,
                profile.sweep.to_degrees(),
                profile.chord
            ),
            None => format!(" area={:.6}", std::f64::consts::PI * radius * radius),
        };
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: None,
            diagnostics: vec![Diagnostic {
                object: Some(ObjectId(entity.id.0)),
                code: "measuregeom.radius".into(),
                message: format!(
                    "MEASUREGEOM RADIUS: entity {} centre={} radius={radius:.6} diameter={diameter:.6} \
                     circumference={circumference:.6}{extra}",
                    entity.id.0,
                    format_point(center),
                ),
            }],
            measurement: None,
            point_report: None,
        })
    }
}

/// The arc-only part of the `MEASUREGEOM RADIUS` report.
struct ArcProfile {
    arc_length: f64,
    /// The signed stored sweep, in radians (reported in degrees).
    sweep: f64,
    chord: f64,
}

/// The AutoCAD spelling of one `MEASUREGEOM` option, for the stable refusals.
fn measure_geom_option_name(option: MeasureGeomOption) -> &'static str {
    match option {
        MeasureGeomOption::Distance => "MEASUREGEOM DISTANCE",
        MeasureGeomOption::Angle => "MEASUREGEOM ANGLE",
        MeasureGeomOption::Area => "MEASUREGEOM AREA",
        MeasureGeomOption::Polyline => "MEASUREGEOM POLYLINE",
        MeasureGeomOption::Radius => "MEASUREGEOM RADIUS",
        MeasureGeomOption::Volume => "MEASUREGEOM VOLUME",
    }
}

fn validate_radius_entity(entity: &cad_db::DbEntity, center: Point3, radius: f64) -> CadResult<()> {
    if !camera::is_finite_point(center) {
        return Err(CadError::InvalidInput(format!(
            "measuregeom.radius.degenerate: entity {} has a non-finite centre",
            entity.id.0
        )));
    }
    if !radius.is_finite() || radius <= 0.0 {
        return Err(CadError::InvalidInput(format!(
            "measuregeom.radius.degenerate: entity {} has radius {radius}, which is not a positive \
             finite length",
            entity.id.0
        )));
    }
    Ok(())
}
