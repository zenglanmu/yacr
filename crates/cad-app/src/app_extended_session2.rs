//! Wave 2 session tokens for the extended command batch.
//!
//! `DRAGMODE`, `APERTURE`, `ISOPLANE`, `HIGHLIGHTNEW`, `TRAYSETTINGS`, `TEXTSCR`,
//! `GRAPHSCR`, `FINDINCLUDE`, `FINDMATCH` and `REVCLOUDPROPERTIES` each read or
//! flip one token in [`SessionState::drafting_extras`]. The commands take no
//! payload, open no transaction, bump no revision and record no `cad-history`
//! entry.
//!
//! `APERTURE <n>` and `REVCLOUDPROPERTIES <length>` are the two **set** forms;
//! their value rides the existing `CommandPayload::LayerWalk` token carrier and
//! is re-validated here (the command-line grammar lives in `cad-ui-core`).
//! `APERTURE` is consumed by the object-snap aperture of the host pick/snap
//! paths ([`crate::SessionState::interaction_tolerance`]); `REVCLOUDPROPERTIES`
//! is consumed by `REVCLOUD` ([`crate::app_drawing::revision_cloud`]). Every
//! other arm here only reports the session value: this build has no drag
//! preview, no isometric cursor, no new-feature highlighting, no status-bar tray,
//! no text-screen window and no cross-block find, and each outcome says exactly
//! that instead of claiming a renderer effect.
//!
//! `ISOPLANE` and `APERTURE`/`REVCLOUDPROPERTIES` are **tokens, not toggles**:
//! `ISOPLANE` cycles `LEFT → TOP → RIGHT`, and the other two report or set a
//! value.

use super::*;
use crate::app_drafting::toggle_outcome;
use crate::app_reports_ext::diagnostic;
use crate::app_utility::require_none_payload;
use crate::session_state::IsoPlane;
use crate::viewer_config::ViewAperture;

impl Application {
    /// Dispatch one Wave 2 session token.
    pub(crate) fn extended_session2_toggle(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        let name = ext.selector();
        match ext {
            ExtendedCommandId::Aperture => return self.aperture_token(session, command),
            ExtendedCommandId::RevCloudProperties => {
                return self.revcloud_properties_token(session, command)
            }
            _ => require_none_payload(command, name)?,
        }
        match ext {
            ExtendedCommandId::DragMode => {
                let enabled = session.drafting_extras.toggle_dragmode();
                Ok(toggle_outcome(
                    "dragmode.toggled",
                    format!(
                        "{name}: session flag is now {} (session state only; the drawing was not \
                         written and drag-preview behaviour is NOT IMPLEMENTED in this build, so \
                         no dragged-image effect is claimed or verified)",
                        state(enabled)
                    ),
                ))
            }
            ExtendedCommandId::IsoPlane => {
                let plane: IsoPlane = session.drafting_extras.cycle_isoplane();
                Ok(toggle_outcome(
                    "isoplane.toggled",
                    format!(
                        "{name}: session isometric plane is now {} (session state only); this build \
                         has no isometric cursor or snap change, so no drafting-plane effect is \
                         claimed or verified",
                        plane.label()
                    ),
                ))
            }
            ExtendedCommandId::HighlightNew => {
                let enabled = session.drafting_extras.toggle_highlightnew();
                Ok(toggle_outcome(
                    "highlightnew.toggled",
                    format!(
                        "{name}: session flag is now {} (session state only); this build renders \
                         no new-feature highlight, so no orange-dot effect is claimed",
                        state(enabled)
                    ),
                ))
            }
            ExtendedCommandId::TraySettings => {
                let enabled = session.drafting_extras.toggle_traysettings();
                Ok(toggle_outcome(
                    "traysettings.toggled",
                    format!(
                        "{name}: session flag is now {} (session state only); this build has no \
                         status-bar tray UI, so no icon or notification change is claimed",
                        state(enabled)
                    ),
                ))
            }
            ExtendedCommandId::TextScreen => {
                session.drafting_extras.set_text_screen(true);
                Ok(toggle_outcome(
                    "textscr.toggled",
                    format!(
                        "{name}: session text-screen token is now ON (session state only); this \
                         build has no text-screen window, so no window is shown and no prompt \
                         history is displayed"
                    ),
                ))
            }
            ExtendedCommandId::GraphScreen => {
                session.drafting_extras.set_text_screen(false);
                Ok(toggle_outcome(
                    "graphscr.toggled",
                    format!(
                        "{name}: session text-screen token is now OFF (session state only); this \
                         build has no text-screen window to hide, so no window change is claimed"
                    ),
                ))
            }
            ExtendedCommandId::FindInclude => {
                let enabled = session.drafting_extras.toggle_find_include();
                Ok(toggle_outcome(
                    "findinclude.toggled",
                    format!(
                        "{name}: session flag is now {} (session state only); cross-block/xref \
                         text search is NOT IMPLEMENTED in this build, so no search scope change \
                         is claimed",
                        state(enabled)
                    ),
                ))
            }
            ExtendedCommandId::FindMatch => {
                let enabled = session.drafting_extras.toggle_find_match_case();
                Ok(toggle_outcome(
                    "findmatch.toggled",
                    format!(
                        "{name}: session flag is now {} (session state only); the implemented \
                         `-FIND`/`FIND` reports always compare ASCII case-insensitively, so this \
                         token does not change their matching",
                        state(enabled)
                    ),
                ))
            }
            other => Err(CadError::Invariant(format!(
                "not a Wave 2 session token: {other:?}"
            ))),
        }
    }

    /// `APERTURE` report or `APERTURE <n>` set.
    ///
    /// The value is re-validated here (whole pixels inside
    /// [`ViewAperture::RANGE`]) even though the command-line grammar already
    /// checked it, so a direct payload cannot store an unusable aperture. A set
    /// writes only the session token: no revision, no history entry. The token
    /// is the object-snap aperture the host pick/snap paths read
    /// ([`SessionState::interaction_tolerance`]); the pickbox is the separate
    /// `PICKBOX`/config value and is never changed by this command.
    fn aperture_token(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::None => Ok(report(
                "aperture.value",
                if session.drafting_extras.aperture_explicit() {
                    format!(
                        "APERTURE: session aperture token is {} logical pixels (used as the \
                         object-snap aperture by the host pick paths; the pickbox is separate)",
                        session.drafting_extras.aperture()
                    )
                } else {
                    format!(
                        "APERTURE: session aperture token is {} logical pixels but has not been \
                         set explicitly, so the configured view.aperture value applies; \
                         `APERTURE <n>` sets the session token",
                        session.drafting_extras.aperture()
                    )
                },
            )),
            CommandPayload::LayerWalk(tokens) => {
                let value = aperture_token_value(tokens)?;
                session.drafting_extras.set_aperture(value);
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![diagnostic(
                        "aperture.set",
                        format!(
                            "APERTURE: session aperture token is now {value} logical pixels (the \
                             object-snap aperture used by the next pick/snap resolution; no \
                             drawing change, no revision, no undo entry)"
                        ),
                    )],
                    measurement: None,
                    point_report: None,
                })
            }
            _ => Err(CadError::InvalidInput(
                "aperture.invalid_argument: APERTURE takes no payload other than one whole pixel \
                 value"
                    .into(),
            )),
        }
    }

    /// `REVCLOUDPROPERTIES` report or `REVCLOUDPROPERTIES <length>` set.
    ///
    /// The value is re-validated here (finite and strictly positive) and written
    /// to the session token `REVCLOUD` already reads when the payload carries no
    /// explicit arc length. A set writes only the session token.
    fn revcloud_properties_token(
        &self,
        session: &mut SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        match &command.payload {
            CommandPayload::None => Ok(report(
                "revcloud.properties",
                format!(
                    "REVCLOUDPROPERTIES: session arc-length token is {} and REVCLOUD reads it \
                     when the command carries no explicit Arc Length",
                    session.drafting_extras.revcloud_arc_length()
                ),
            )),
            CommandPayload::LayerWalk(tokens) => {
                let value = revcloud_properties_token_value(tokens)?;
                session.drafting_extras.set_revcloud_arc_length(value);
                Ok(CommandOutcome {
                    objects: Vec::new(),
                    changes: None,
                    diagnostics: vec![diagnostic(
                        "revcloud.properties.set",
                        format!(
                            "REVCLOUDPROPERTIES: session arc-length token is now {value}; the next \
                             REVCLOUD without an explicit Arc Length uses it (no drawing change, \
                             no revision, no undo entry)"
                        ),
                    )],
                    measurement: None,
                    point_report: None,
                })
            }
            _ => Err(CadError::InvalidInput(
                "revcloudproperties.invalid_argument: REVCLOUDPROPERTIES takes no payload other \
                 than one finite positive arc length"
                    .into(),
            )),
        }
    }
}

/// The one `APERTURE` value carried by the token payload, re-validated.
fn aperture_token_value(tokens: &[String]) -> CadResult<u32> {
    let [token] = tokens else {
        return Err(CadError::InvalidInput(
            "aperture.invalid_argument: APERTURE takes exactly one whole pixel value".into(),
        ));
    };
    let text = token.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CadError::InvalidInput(format!(
            "aperture.invalid_argument: {:?} is not a whole pixel value",
            text
        )));
    }
    let value: u32 = text.parse().map_err(|_| {
        CadError::InvalidInput(format!(
            "aperture.invalid_argument: {:?} is not a whole pixel value",
            text
        ))
    })?;
    if !ViewAperture::RANGE.contains(&value) {
        return Err(CadError::InvalidInput(format!(
            "aperture.out_of_range: APERTURE must be {}..={} logical pixels, got {value}",
            ViewAperture::RANGE.start(),
            ViewAperture::RANGE.end()
        )));
    }
    Ok(value)
}

/// The one `REVCLOUDPROPERTIES` value carried by the token payload, re-validated.
fn revcloud_properties_token_value(tokens: &[String]) -> CadResult<f64> {
    let [token] = tokens else {
        return Err(CadError::InvalidInput(
            "revcloudproperties.invalid_argument: REVCLOUDPROPERTIES takes exactly one arc \
             length"
                .into(),
        ));
    };
    let text = token.trim();
    let value: f64 = text.parse().map_err(|_| {
        CadError::InvalidInput(format!(
            "revcloudproperties.invalid_argument: {:?} is not an arc length",
            text
        ))
    })?;
    if !value.is_finite() || value <= 0.0 {
        return Err(CadError::InvalidInput(format!(
            "revcloudproperties.invalid_argument: the arc length must be finite and positive, \
             got {value}"
        )));
    }
    Ok(value)
}

fn state(enabled: bool) -> &'static str {
    if enabled {
        "ON"
    } else {
        "OFF"
    }
}

fn report(code: &str, message: String) -> CommandOutcome {
    CommandOutcome {
        objects: Vec::new(),
        changes: None,
        diagnostics: vec![diagnostic(code, message)],
        measurement: None,
        point_report: None,
    }
}
