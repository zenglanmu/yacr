//! Session display toggles for the extended command batch.
//!
//! `QTEXT`, `FILL` and `TFRAMES` are **display tokens with real consumers**: the
//! flag each command flips is pushed into the scene controller
//! ([`crate::render_scene::DisplaySettings`]) and read by the representation
//! provider, so the derived display really changes — `FILL OFF` draws filled
//! hatches/2D solids/wide polylines as outlines, `QTEXT ON` draws shaped text as
//! a placeholder rectangle, and `TFRAMES OFF` hides the raster-image boundary
//! frame and the WIPEOUT boundary frame. None writes `cad-db`: no transaction,
//! no revision bump, no `cad-history` entry, no `ChangeSet`, no undo step.
//!
//! `REGENAUTO` and `ISODRAFT` have no display consumer in this build (there is
//! no automatic-regeneration scheduler and no isometric cursor/snap model), so
//! those two still store the token and say exactly that instead of claiming a
//! visual change.

use super::*;
use crate::app_utility::require_none_payload;

pub(crate) fn toggle_outcome(code: &str, message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![Diagnostic {
            object: None,
            code: code.into(),
            message,
        }],
        measurement: None,
        point_report: None,
    }
}

impl Application {
    /// Toggle one drafting flag (`QTEXT`/`FILL`/`TFRAMES`/`REGENAUTO`/`ISODRAFT`).
    ///
    /// The three display tokens report what the display pipeline now does with the
    /// value; the two without a consumer say their effect is NOT IMPLEMENTED.
    /// In every case the drawing is untouched.
    pub(crate) fn drafting_toggle(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        require_none_payload(command, name)?;
        let (code, enabled, behaviour) = match ext {
            ExtendedCommandId::QText => {
                let enabled = session.drafting_extras.toggle_qtext();
                let state = if enabled { "ON" } else { "OFF" };
                let behaviour = if enabled {
                    "every text entity now draws as the placeholder rectangle of its shaped \
                     extent (the glyphs are still shaped, so this build claims no redraw-speed \
                     gain); text with no derivable extent keeps its own draw and reports \
                     qtext.extent_unknown"
                } else {
                    "text entities draw their shaped glyphs again"
                };
                return Ok(toggle_outcome(
                    "qtext.toggled",
                    format!(
                        "{name}: quick-text display is now {state} (session state only; the \
                         drawing was not written): {behaviour}"
                    ),
                ));
            }
            ExtendedCommandId::Fill => {
                let enabled = session.drafting_extras.toggle_fill();
                let state = if enabled { "ON" } else { "OFF" };
                let behaviour = if enabled {
                    "filled hatches, 2D solids and wide polylines draw filled again; the stored \
                     geometry was unchanged throughout"
                } else {
                    "filled hatches, 2D solids and wide polylines now draw their outline only \
                     (hatch boundary loops, the solid's boundary edges, the stroke band's closed \
                     offset outline); the entity geometry itself is unchanged, and this build \
                     does not switch a bare kernel/B-rep display mesh, a dimension arrowhead, a \
                     WIPEOUT mask or an image"
                };
                return Ok(toggle_outcome(
                    "fill.toggled",
                    format!(
                        "{name}: solid-fill display is now {state} (session state only; the \
                         drawing was not written): {behaviour}"
                    ),
                ));
            }
            ExtendedCommandId::TFrames => {
                let enabled = session.drafting_extras.toggle_tframes();
                let state = if enabled { "ON" } else { "OFF" };
                let behaviour = if enabled {
                    "raster images draw their boundary frame again, and WIPEOUT masks draw the \
                     boundary frame their own WIPEOUTFRAME token allows; no drawing data changed"
                } else {
                    "the raster-image boundary frame and (regardless of WIPEOUTFRAME) the WIPEOUT \
                     mask boundary frame are hidden; a WIPEOUT still masks and an image still \
                     draws its texture, and no drawing data changed"
                };
                return Ok(toggle_outcome(
                    "tframes.toggled",
                    format!(
                        "{name}: frame display is now {state} (session state only; the drawing \
                         was not written): {behaviour}. External-reference frames are not \
                         modelled in this build (an xref is expanded block geometry with no frame \
                         to toggle), and a texture-less image keeps its placeholder frame plus \
                         its image.* diagnostic"
                    ),
                ));
            }
            ExtendedCommandId::Edges => {
                let enabled = session.drafting_extras.toggle_body_edges();
                let state = if enabled { "ON" } else { "OFF" };
                let behaviour = if enabled {
                    "shaded 3D bodies draw their stored/topology edge overlay again; no drawing                      data changed"
                } else {
                    "shaded 3D bodies keep their fill but draw no edge overlay, and a wire-only                      body (no triangles) draws nothing; standalone line entities are not hidden                      by this body-edge token, and no drawing data changed"
                };
                return Ok(toggle_outcome(
                    "edges.toggled",
                    format!(
                        "{name}: body edge display is now {state} (session state only; the                          drawing was not written): {behaviour}"
                    ),
                ));
            }
            ExtendedCommandId::RegenAuto => (
                "regenauto.toggled",
                session.drafting_extras.toggle_regenauto(),
                "automatic regeneration",
            ),
            ExtendedCommandId::IsoDraft => (
                "isodraft.toggled",
                session.drafting_extras.toggle_isodraft(),
                "isometric-plane drafting",
            ),
            other => {
                return Err(CadError::Invariant(format!(
                    "not a drafting toggle: {other:?}"
                )))
            }
        };
        let state = if enabled { "ON" } else { "OFF" };
        Ok(toggle_outcome(
            code,
            format!(
                "{name}: session flag is now {state} (session state only; the drawing was not \
                 written and {behaviour} is NOT IMPLEMENTED in this build, so no visual effect is \
                 claimed or verified)"
            ),
        ))
    }
}
