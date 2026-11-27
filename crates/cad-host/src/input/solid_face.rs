//! Tool start for the one-pick solid face edits (`CHAMFEREDGE` / `PRESSPULL`).
//!
//! Split out of `interaction.rs` (code-size limit) so the start rules and the
//! Push/Pull-on-a-profile route live next to `face_tool.rs`'s command table.

use super::*;

impl HostCore {
    /// Start a one-pick **solid face edit** capture (`CHAMFEREDGE` or
    /// `PRESSPULL`) after failing fast on its prerequisites.
    ///
    /// The order is the command path's order: **authorize first** (the extended
    /// command is Work-only, so a Viewer session gets the same
    /// `PermissionDenied` a typed command would), then the carrier capability
    /// (`unsupported-runtime: solid-modeler`), then the selection (the shared
    /// target resolver, with the command's own `*.selection_required`-family
    /// codes).
    ///
    /// # Push/Pull on a drawn profile
    ///
    /// SketchUp's Push/Pull starts on a **selected closed planar profile** too:
    /// a rectangle or circle has no solid face to pick yet, so the first pull
    /// creates the solid. That path arms [`cad_app::DrawToolKind::Extrude`]
    /// instead of the face capture: it is the same `EXTRUDE` command (one
    /// transaction, one undo entry, the same named refusals) reached through the
    /// tool the user pressed. The profile predicate only decides the route; the
    /// command validates the same rows again at execute time, so a document that
    /// changed between the tool start and Enter still refuses by name instead of
    /// extruding something else.
    ///
    /// All of it runs before the session arms a capture, so a refused start
    /// leaves no tool and consumes no click.
    pub(super) fn begin_solid_face_tool(&self, kind: cad_app::DrawToolKind) -> CadResult<()> {
        let pick = solid_face_tool(kind)?;
        {
            let c = self.controller.borrow();
            c.session.authorize(pick.command)?;
        }
        let (has_modeler, document, refs) = {
            let c = self.controller.borrow();
            (
                c.application.has_solid_modeler(),
                c.document_id,
                c.session.selection.refs().to_vec(),
            )
        };
        // P-MESH-2: a PRESSPULL on a mesh face is a kernel-free topology edit, so
        // it does not need the exact-solid modeler or the exact-body target
        // resolver. It still needs a real selected face id.
        let mesh_press_pull = kind == cad_app::DrawToolKind::PressPull
            && refs.len() == 1
            && refs[0].sub_element.is_some()
            && refs[0].instance.0.is_empty()
            && {
                let c = self.controller.borrow();
                c.application
                    .workspace
                    .documents
                    .get(&document)
                    .and_then(|document| document.drawing.entity(refs[0].entity))
                    .is_some_and(mesh_face_edit_target)
            };
        if !has_modeler && !mesh_press_pull {
            return Err(CadError::Unsupported(
                cad_app::SOLID_MODELER_UNSUPPORTED.into(),
            ));
        }
        // A closed profile is the one Push/Pull start with no face to pick: the
        // extruder builds the solid and leaves the drawing as the durable input.
        let profile_press_pull = kind == cad_app::DrawToolKind::PressPull && {
            let c = self.controller.borrow();
            c.application.closed_profile_selection(document, &refs)
        };
        if profile_press_pull {
            return self
                .controller
                .borrow_mut()
                .begin_draw_tool(cad_app::DrawToolKind::Extrude);
        }
        if !mesh_press_pull {
            let c = self.controller.borrow();
            (pick.target)(&c.application, document, &refs)?;
        }
        self.controller.borrow_mut().begin_draw_tool(kind)
    }
}
