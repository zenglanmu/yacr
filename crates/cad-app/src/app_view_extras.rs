//! The 2026-11-07 extras batch view-only commands: `VIEWRES`, `VSCURRENT`,
//! `IBL`, `WIPEOUTFRAME` and `CVPORT`.
//!
//! All of them mutate [`SessionState`] only: no database write, revision bump,
//! `cad-history` entry, `ChangeSet` or `ZOOM PREVIOUS` snapshot. `-PAN` is
//! not implemented here: it rides the existing `CommandId::Pan` camera path with
//! a `Points` delta, so it has no second camera authority.
//!
//! `VSCURRENT` stores the session 3D-body visual style and it **reaches the
//! display**: `Wireframe` becomes
//! [`cad_representation::RepresentationContext::body_wireframe`], the one flag
//! the body fragment builder reads. No pixel/GPU effect is claimed or verified;
//! the flag changes only `Body3d` display, never drawing data.
//!
//! `WIPEOUTFRAME` is **not** a passive token: the display scene and the
//! vector-plot context both read it (`cad-representation`'s
//! `RepresentationContext::wipeout_frames`), which is what makes the
//! `WIPEOUTFRAME 2` display/plot split real.
//!
//! `CVPORT` is **not** passive either: `LAYVPI`, `VPSCALE` and `VPCLIP` target
//! the viewport it names. See [`crate::command_id::extras`] for the exact
//! per-layout/session-only contract.

use super::*;

use crate::app_layout::{layout_viewports, require_paper_space};

fn view_outcome(code: &str, message: String) -> CommandOutcome {
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
    /// `VIEWRES <1..=1000>`: store the session circle-tessellation percentage.
    ///
    /// The range is re-validated here even though the typed parser already
    /// enforces it: a direct IPC caller gets the same named refusal, never a
    /// clamp.
    ///
    /// The stored percentage is **consumed**: the display build turns it into a
    /// ceiling on chord segments per curve
    /// ([`cad_representation::viewres_segment_ceiling`]), so lowering it really
    /// coarsens the curves that are chord-tessellated (splines, polyline bulge
    /// arcs, the chord fallback for a non-SDF conic, the wide-polyline band).
    /// Two honest limits are reported with the value: circles/arcs/ellipses are
    /// drawn as analytic conics and have no chords to bound, and the value is a
    /// ceiling on the display tolerance rather than a smoothness setting, so
    /// raising it above what the tolerance already asks for changes nothing.
    pub(crate) fn viewres(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::ViewRes(percent) = &command.payload else {
            return Err(CadError::InvalidInput(
                "VIEWRES needs one integer percentage in 1..=1000".into(),
            ));
        };
        let percent = *percent;
        session.view_extras.set_viewres(percent)?;
        let ceiling = cad_representation::viewres_segment_ceiling(percent);
        Ok(view_outcome(
            "viewres",
            format!(
                "VIEWRES: circle tessellation set to {percent}% (session state only; the drawing \
                 was not written) — the display build now caps chord-tessellated curves at \
                 {ceiling} segment(s) each, so splines, polyline bulge arcs, chord-fallback conics \
                 and wide-polyline bands follow it; circles, arcs and ellipses are drawn as \
                 analytic conics and are unaffected, and because the value is a ceiling on the \
                 display tolerance any value above what the tolerance already needs changes \
                 nothing"
            ),
        ))
    }

    /// `VSCURRENT <style>`: store the session visual-style token.
    ///
    /// Accepted spellings are `WIREFRAME`/`2DWIREFRAME`/`2D` (3D bodies draw
    /// their edges only) and `SHADED`/`SHADEDWITHEDGES` (filled body meshes plus
    /// stored edges, the session default). Every other style is refused by name
    /// (`vscurrent.style_not_implemented`), never silently mapped.
    ///
    /// Two divergences are declared, not silently papered over: the `SHADED`
    /// token **includes** the stored-edge overlay (AutoCAD's shaded style has no
    /// edges; this build has no separate edge-free token), and a curved body in
    /// `WIREFRAME` shows only its stored edge loops — there is no silhouette
    /// generator, so the outer contour can differ from AutoCAD's.
    ///
    /// The token is session-only and affects **3D bodies only**: `DisplaySettings`
    /// turns `Wireframe` into
    /// [`cad_representation::RepresentationContext::body_wireframe`], whose sole
    /// consumer is the body fragment builder. No other geometry changes and the
    /// drawing is never written.
    pub(crate) fn vscurrent(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::VisualStyle(style) = &command.payload else {
            return Err(CadError::InvalidInput(
                "VSCURRENT needs one style name (WIREFRAME, 2DWIREFRAME, 2D, SHADED or \
                 SHADEDWITHEDGES)"
                    .into(),
            ));
        };
        session.view_extras.set_visual_style(style)?;
        let active = session.view_extras.visual_style;
        Ok(view_outcome(
            "vscurrent",
            format!(
                "VSCURRENT: visual style is now {} (session state only; the drawing was not \
                 written and no revision or history entry was recorded) — {}",
                active.name(),
                match active {
                    VisualStyleToken::Wireframe =>
                        "3D bodies draw their edge segments only (stored body edges, or \
                         triangle-derived segments when a primitive has no stored boundary \
                         edges); other geometry keeps its display",
                    VisualStyleToken::Shaded =>
                        "3D bodies draw their filled mesh plus stored-edge overlay; other geometry \
                         keeps its display",
                }
            ),
        ))
    }

    /// `IBL [ON|OFF]`: toggle or store the session PBR image-based-lighting
    /// switch.
    ///
    /// A **bare** `IBL` toggles the current value; `IBL ON`/`IBL OFF` (and the
    /// typed `1`/`0` spellings, parsed by the command line) set it explicitly.
    /// The stored token is [`PbrEnvironment`], the value the procedural
    /// image-based-lighting lane reads at render time.
    ///
    /// Session-only and display-only: no database write, revision bump,
    /// `cad-history` entry, `ChangeSet` or persisted preference. A payload that
    /// is not this command's own is refused rather than treated as a toggle.
    pub(crate) fn ibl(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let enabled = match &command.payload {
            // The bare selector: flip the live value.
            CommandPayload::None => !session.view_extras.pbr_environment.is_image_based(),
            CommandPayload::Ibl(enabled) => *enabled,
            _ => {
                return Err(CadError::InvalidInput(
                    "ibl.invalid_payload: IBL takes ON or OFF (or no payload to toggle)".into(),
                ))
            }
        };
        session
            .view_extras
            .set_pbr_environment(if enabled { "IBL" } else { "LEGACY" })?;
        let active = session.view_extras.pbr_environment;
        Ok(view_outcome(
            "ibl.value",
            format!(
                "IBL: image-based lighting is {} ({}) — {} (session state only; the drawing was \
                 not written and no revision or history entry was recorded)",
                if active.is_image_based() { "ON" } else { "OFF" },
                active.name(),
                if active.is_image_based() {
                    "PBR metal/roughness surfaces are lit by the procedural environment"
                } else {
                    "the shader falls back to the single dominant light"
                }
            ),
        ))
    }

    /// `WIPEOUTFRAME [0|1|2]`: report or store the session frame state.
    ///
    /// This is the **same** token the `WIPEOUT Frames ON|OFF|Display` option and
    /// the drawing-settings panel write ([`SessionState::drafting_extras`]); the
    /// three paths share one setter, so a value set here is what the display
    /// scene and the vector-plot context read. No payload is the report form.
    ///
    /// Display-only: no database write, revision bump, `cad-history` entry or
    /// `ChangeSet`. A payload that is not this command's own is refused rather
    /// than treated as a report.
    pub(crate) fn wipeout_frame(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            // The bare selector: report the live value, change nothing.
            CommandPayload::None => {}
            CommandPayload::WipeoutFrame(frames) => {
                session.drafting_extras.set_wipeout_frames(*frames);
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "wipeoutframe.invalid_payload: WIPEOUTFRAME takes the session frame state (or \
                     no payload for the report)"
                        .into(),
                ))
            }
        }
        let current = session.drafting_extras.wipeout_frames();
        Ok(view_outcome(
            "wipeoutframe.value",
            format!(
                "WIPEOUTFRAME = {} ({}): {} (session state only; the drawing was not written)",
                current.value(),
                current.label(),
                match current {
                    WipeoutFrames::Off =>
                        "every wipeout frame is hidden on screen and in plots; the mask fill still \
                         masks",
                    WipeoutFrames::On =>
                        "wipeout frames are displayed on screen and plotted by the vector output",
                    WipeoutFrames::DisplayNotPlot =>
                        "wipeout frames are displayed on screen but omitted from the vector plot",
                }
            ),
        ))
    }

    /// `CVPORT [<n>]`: report or store the session current-viewport marker.
    ///
    /// The marker is the **one-based** position of the current viewport inside
    /// the active paper layout's stored viewport list (AutoCAD's numbering); the
    /// session stores the zero-based index. It is per layout and session-only:
    /// no database write, revision bump, `cad-history` entry, undo step or
    /// `ChangeSet` — this handler never touches `self.drawing` mutably.
    ///
    /// `LAYVPI`, `VPSCALE` and `VPCLIP` read the same marker, so the reported
    /// value is exactly the viewport those commands will target.
    ///
    /// Refusals (nothing stored, nothing drawn):
    ///
    /// * `cvport.paper_space_required` — the active space is model space (this
    ///   build keeps no model-space viewport id and reports no fake `1`);
    /// * `cvport.no_viewports` — the active layout has no viewport at all;
    /// * `cvport.out_of_range` — the requested/stored position is outside
    ///   `1..=<viewport count>`; never clamped;
    /// * `cvport.invalid_payload` — a payload that is not this command's own.
    pub(crate) fn cvport(
        &mut self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let layout_id = require_paper_space(session, "cvport")?;
        let viewports = {
            let drawing = self.drawing(command.document)?;
            layout_viewports(drawing, layout_id)?
        };
        let count = viewports.len();
        if count == 0 {
            return Err(CadError::Unsupported(format!(
                "cvport.no_viewports: layout {} has no viewport; CVPORT has no current viewport to \
                 report and this build never reports a fabricated 1 (create one with VPORTS or \
                 MVIEW first)",
                layout_id.0
            )));
        }
        match &command.payload {
            // Bare selector: report the stored marker, change nothing.
            CommandPayload::None => {}
            CommandPayload::Cvport(position) => {
                // The command line is one-based; the stored marker is not (`0`
                // is not a valid position, so it is not a silent alias for the
                // first viewport either).
                let position = *position as usize;
                if position == 0 || position > count {
                    return Err(CadError::InvalidInput(format!(
                        "cvport.out_of_range: layout {} has {count} viewport(s); CVPORT takes a \
                         viewport position in 1..={count}, got {position} and nothing was changed",
                        layout_id.0
                    )));
                }
                session.set_viewport_marker(layout_id, position - 1);
            }
            _ => {
                return Err(CadError::InvalidInput(
                    "cvport.invalid_payload: CVPORT takes one viewport position (or no payload \
                     for the report)"
                        .into(),
                ))
            }
        }
        // A stale marker refuses here too instead of reporting a viewport that
        // no command would target. The code is `cvport.out_of_range` (the same
        // family the set form uses), never a clamp to the first viewport.
        let marker = session.viewport_marker(layout_id);
        if marker >= count {
            return Err(CadError::InvalidInput(format!(
                "cvport.out_of_range: the session CVPORT marker names viewport {} (zero-based \
                 index {marker}), but layout {} has {count} viewport(s); set it again with \
                 `CVPORT <n>` (1..={count}) — nothing was changed and nothing was clamped",
                marker + 1,
                layout_id.0
            )));
        }
        Ok(view_outcome(
            "cvport.value",
            format!(
                "CVPORT = {} of {count}: the current viewport of layout {} is its viewport at \
                 zero-based index {marker}; LAYVPI (no index) freezes in every other viewport, and \
                 VPSCALE/VPCLIP (no index) target this one (session state only; the drawing was not \
                 written and no revision, history entry or undo step was recorded)",
                marker + 1,
                layout_id.0
            ),
        ))
    }
}
