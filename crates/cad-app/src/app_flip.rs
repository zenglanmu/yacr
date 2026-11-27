//! `FLIP X|Y|Z`: SketchUp's in-place Flip Along.
//!
//! The reflection plane goes through the **selection's world bounds centre** and
//! is perpendicular to the chosen axis; the sources are erased and their mirrored
//! copies inserted in the same transaction (one revision, one undo step), exactly
//! like a mirror with `Erase source`. The selection follows the new rows, and the
//! command is Work-only.

use super::*;
use cad_domain::Transform3;

impl Application {
    /// `FLIP <axis>`: reflect the selection in place about its bounds centre.
    pub(crate) fn flip_selection(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::Flip(axis) = command.payload else {
            return Err(CadError::InvalidInput(
                "flip.axis_required: FLIP takes X, Y or Z / RED, GREEN or BLUE".into(),
            ));
        };
        if session.app_mode() == AppMode::Viewer {
            return Err(CadError::PermissionDenied);
        }
        let refs = session.selection.refs().to_vec();
        if refs.is_empty() {
            return Err(CadError::InvalidInput(
                "flip.selection_required: FLIP mirrors the current selection; select at least one \
                 object"
                    .into(),
            ));
        }
        let (min, max) =
            self.selected_object_bounds(session, command)
                .map_err(|error| match error {
                    CadError::InvalidInput(text)
                        if text.starts_with("zoom.object.empty_selection:") =>
                    {
                        CadError::InvalidInput(format!("flip.selection_required: {text}"))
                    }
                    other => other,
                })?;
        let centre = Point3 {
            x: (min.x + max.x) / 2.0,
            y: (min.y + max.y) / 2.0,
            z: (min.z + max.z) / 2.0,
        };
        let component = axis.component();
        let centre_components = [centre.x, centre.y, centre.z];
        let mut matrix = Transform3::identity().matrix;
        matrix[component][component] = -1.0;
        // Column-major: `x' = -x + 2c` for the flipped component.
        matrix[component][3] = 2.0 * centre_components[component];
        let transform = Transform3 { matrix };
        let mut outcome =
            self.mirror_selection_with_options(command, &refs, &transform, true, true)?;
        // The in-place flip leaves the new rows selected, so a second FLIP keeps
        // acting on the same object instead of an empty/stale set.
        let selected: Vec<SelectionRef> = outcome
            .objects
            .iter()
            .map(|object| SelectionRef {
                document: command.document,
                entity: EntityId(object.0),
                instance: InstancePath::default(),
                sub_element: None,
            })
            .collect();
        if !selected.is_empty() {
            session.selection = SelectionSet::from_refs(selected);
        }
        outcome.diagnostics.push(Diagnostic {
            object: None,
            code: "flip.applied".into(),
            message: format!(
                "FLIP {}: reflected the selection about the plane through ({:.4}, {:.4}, {:.4})",
                axis.token(),
                centre.x,
                centre.y,
                centre.z
            ),
        });
        Ok(outcome)
    }
}
