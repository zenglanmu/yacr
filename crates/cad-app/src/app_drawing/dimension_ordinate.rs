//! DIMORDINATE create command (ADR 0017 decision C, P5e).
//!
//! The command takes a **feature point** and a **leader endpoint**. The axis is
//! chosen by the documented auto rule — *"if the Y difference is greater, measure
//! the X ordinate"* — comparing the feature and leader points. Autodesk does not
//! state the equal case; **our declared reading is Y** (the documented rule's
//! "otherwise" branch), and the option `OrdinateDimensionCommand::axis` can force
//! either axis. The interactive tool never forces one.
//!
//! The measurement is the feature's **datum-relative** (world-origin-relative)
//! X or Y coordinate **magnitude**, the same semantics P5b pinned for imported
//! ordinate dimensions: `Ordinate` writes group 10 as the datum `(0,0,0)` and
//! `ordinate_synthesis` takes the projection's absolute value, matching the
//! acadrust writer's group-42 recomputation. **Signed ordinates are not
//! modelled** (declared). A feature whose coordinate is 0 is valid, so unlike the
//! linear/angular commands a zero measurement is not a refusal.
//!
//! Every refusal is explicit and opens no transaction; a created dimension is
//! non-associative and lands on the active layer in one transaction / one undo
//! step.

use super::*;

/// The minimum XY extent of the leader for it to define a direction.
const MIN_ORDINATE_LEADER: f64 = 1e-9;

impl Application {
    /// DIMORDINATE: an ordinate dimension for a feature point and leader endpoint
    /// (ADR 0017 decision C, P5e).
    pub(crate) fn dim_ordinate(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::DimOrdinate(input) = &command.payload else {
            return Err(CadError::InvalidInput(
                "DIMORDINATE needs a DimOrdinate(feature, leader, axis) payload".into(),
            ));
        };
        let feature = input.feature;
        let leader = input.leader;
        if !is_finite_point(feature) || !is_finite_point(leader) {
            return Err(CadError::InvalidInput(
                "DIMORDINATE points must both be finite".into(),
            ));
        }
        // Flat +Z subset (ADR 0017 D.6): the leader must lie in the feature's
        // plane; an off-plane pair is refused rather than guessed onto a work
        // plane.
        let z_tol = 1e-9 * (1.0 + feature.z.abs() + leader.z.abs());
        if (leader.z - feature.z).abs() > z_tol {
            return Err(CadError::InvalidInput(
                "DIMORDINATE points must lie in one +Z plane; off-plane dimension creation is \
                 NOT IMPLEMENTED"
                    .into(),
            ));
        }
        if planar_distance(feature, leader) <= MIN_ORDINATE_LEADER {
            return Err(CadError::InvalidInput(
                "DIMORDINATE needs a leader endpoint distinct from the feature point".into(),
            ));
        }
        let axis = input
            .axis
            .unwrap_or_else(|| auto_ordinate_axis(feature, leader));
        let definition = DimensionDefinition {
            kind: DimensionKind::Ordinate {
                x_ordinate: axis == OrdinateAxis::X,
            },
            points: DimensionPoints::Ordinate {
                feature,
                leader_end: leader,
                // Group 10 is the creation datum: the world origin. P5b's
                // ordinate semantics measure the feature relative to it and take
                // the magnitude (signed ordinates are not modelled).
                base: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
            // Group 11/53 stay at the zero "derive from the leader" sentinel.
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
        let style = self
            .workspace
            .documents
            .get(&command.document)
            .ok_or_else(|| CadError::InvalidInput("document not open".into()))?
            .drawing
            .resolve_dim_style("Standard")
            .ok_or_else(|| {
                CadError::InvalidInput(
                    "the drawing has no 'Standard' dimension style to create with".into(),
                )
            })?;
        let dimension =
            DimensionGeometry::synthesized(definition, "Standard".to_string(), style, None)?;
        self.commit_new_entity(
            session,
            command,
            SemanticGeometry::Dimension(Box::new(dimension)),
            "create dimordinate",
        )
    }
}

/// The documented auto X/Y rule (ADR 0017 U-5): a greater Y difference between the
/// feature and leader points measures the **X** ordinate; otherwise (including
/// the unstated **equal** case, our declared choice) it measures **Y**.
fn auto_ordinate_axis(feature: Point3, leader: Point3) -> OrdinateAxis {
    if (leader.y - feature.y).abs() > (leader.x - feature.x).abs() {
        OrdinateAxis::X
    } else {
        OrdinateAxis::Y
    }
}

/// The XY distance between two points.
fn planar_distance(a: Point3, b: Point3) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64) -> Point3 {
        Point3 { x, y, z: 0.0 }
    }

    /// U-5: the documented rule, plus our declared equal-case reading.
    #[test]
    fn auto_axis_matches_the_documented_rule_and_the_equal_case() {
        // A greater Y difference measures X.
        assert_eq!(
            auto_ordinate_axis(at(0.0, 0.0), at(1.0, 5.0)),
            OrdinateAxis::X
        );
        assert_eq!(
            auto_ordinate_axis(at(3.0, -2.0), at(4.0, -9.0)),
            OrdinateAxis::X
        );
        // A greater X difference measures Y.
        assert_eq!(
            auto_ordinate_axis(at(0.0, 0.0), at(5.0, 1.0)),
            OrdinateAxis::Y
        );
        // Equal differences are unstated by Autodesk; our declared choice is Y.
        assert_eq!(
            auto_ordinate_axis(at(0.0, 0.0), at(4.0, 4.0)),
            OrdinateAxis::Y
        );
    }
}
