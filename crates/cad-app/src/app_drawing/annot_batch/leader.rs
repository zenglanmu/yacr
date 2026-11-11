//! `QLEADER` / `LEADER`: the legacy leader names mapped onto the existing
//! `MLEADER` creation path, plus the **one shared creation option plan**.
//!
//! Both commands build the ordinary [`CommandPayload::MLeader`] payload and
//! call [`Application::create_mleader`], so there is exactly one construction
//! point (style resolution, geometry policy, text validation and the
//! one-transaction / one-undo funnel). No `AcDbLeader` entity is created.
//!
//! ## Options (AutoCAD's `QLEADER` settings, where this build can represent them)
//!
//! The typed grammar and the payload carry **no option fields**
//! (`<arrow> <landing> <text>` and `MLeader { arrow, landing, text }`), and this
//! change is not allowed to add payload or domain fields. So the options are
//! resolved from the drawing's `MLEADERSTYLE` row — the one existing authority for
//! leader look — by [`plan_leader_options`], which every one of `MLEADER`,
//! `QLEADER` and `LEADER` runs through:
//!
//! | AutoCAD option | Representation | Behaviour |
//! |---|---|---|
//! | leader line type (straight / spline) | style `173` (`MLeaderPathType`) | honoured: a spline style creates a spline-path leader |
//! | maximum points | line vertex list | declared limit [`MAX_CREATED_LEADER_POINTS`]; a request above it is refused by name |
//! | annotation type (text) | style `170` content kind | honoured: MText content only, block/tolerance/none refused by name |
//! | arrowhead on/off | style `44` arrowhead size | an arrowhead-off row (`0`) is refused by name instead of silently drawing one |
//!
//! The refusal codes are the shared `mleader.*` ones: there is exactly one
//! construction point, so `QLEADER`/`LEADER` deliberately do **not** fork the
//! option code namespace (their own prefix only covers their typed-argument
//! grammar: `qleader.payload`, `qleader.invalid_point`, …).

use super::*;
use cad_db::{MLeaderStyle, MLeaderStyleResolution};
use cad_domain::{MLeaderContent, MLeaderPathType};

/// The number of vertices a created leader derives.
///
/// [`MultiLeaderGeometry::created`] is the single derivation: vertex 1 is the
/// captured arrow position and vertex 2 the style-derived dogleg start. The
/// payload has no vertex list, so no caller can request more; this constant and
/// [`plan_leader_options`]'s refusal exist so that the limit is **named** rather
/// than implied, and so a future multi-vertex payload cannot silently grow an
/// unbounded line.
pub(crate) const MAX_CREATED_LEADER_POINTS: usize = 2;

/// The effective creation options of one leader, after validation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LeaderOptions {
    /// Leader line type: `StraightLineSegments` or `Spline`.
    pub(crate) path: MLeaderPathType,
    /// The vertex count the created line will have.
    pub(crate) max_points: usize,
    /// Whether the created leader draws the style arrowhead (always `true`: an
    /// arrowhead-off row is refused).
    pub(crate) arrowhead: bool,
}

/// Resolve and validate the creation options from the drawing's style row.
///
/// `row` is the **stored** row (the raw `arrowhead_size` matters: the resolved
/// snapshot clamps a `0` to the pinned default, which would hide an
/// arrowhead-off style), `resolution` the single resolved snapshot the created
/// entity stores.
///
/// Refusals (named, nothing written):
///
/// * `mleader.content_unsupported` — block/tolerance annotation content;
/// * `mleader.text_required` — a content-less style (no annotation type);
/// * `mleader.option_arrowhead_off` — the row disables the arrowhead (size `0`);
/// * `mleader.path_type` — an invisible leader path (no line at all);
/// * `mleader.option_max_points` — a requested vertex count above
///   [`MAX_CREATED_LEADER_POINTS`];
/// * `mleader.style_invalid` — a non-finite arrowhead size.
pub(crate) fn plan_leader_options(
    row: &MLeaderStyle,
    resolution: &MLeaderStyleResolution,
) -> CadResult<LeaderOptions> {
    let annotation = match row.content_type {
        MLeaderStyle::CONTENT_MTEXT => MLeaderContent::Text,
        MLeaderStyle::CONTENT_NONE => {
            return Err(CadError::InvalidInput(
                "mleader.text_required: the resolved MLEADERSTYLE declares no content; a \
                 content-less MULTILEADER is only imported, displayed and exported"
                    .into(),
            ))
        }
        MLeaderStyle::CONTENT_BLOCK | MLeaderStyle::CONTENT_TOLERANCE => {
            return Err(CadError::Unsupported(format!(
                "mleader.content_unsupported: the resolved MLEADERSTYLE declares content kind {} \
                 (block/tolerance); only MText content is created",
                row.content_type
            )))
        }
        other => {
            return Err(CadError::Unsupported(format!(
                "mleader.content_unsupported: the resolved MLEADERSTYLE declares unknown content \
                 kind {other}; only MText content is created"
            )))
        }
    };
    debug_assert_eq!(annotation, MLeaderContent::Text);
    check_annotation_type(annotation)?;
    if !row.arrowhead_size.is_finite() {
        return Err(CadError::InvalidInput(format!(
            "mleader.style_invalid: the arrowhead size must be finite, got {}",
            row.arrowhead_size
        )));
    }
    if row.arrowhead_size <= 0.0 {
        return Err(CadError::Unsupported(format!(
            "mleader.option_arrowhead_off: the resolved MLEADERSTYLE disables the arrowhead \
             (size {}); creation always draws the style arrowhead, so nothing was written",
            row.arrowhead_size
        )));
    }
    let path =
        match resolution.values.path_type {
            MLeaderPathType::StraightLineSegments => MLeaderPathType::StraightLineSegments,
            MLeaderPathType::Spline => MLeaderPathType::Spline,
            MLeaderPathType::Invisible => return Err(CadError::Unsupported(
                "mleader.path_type: the resolved style declares an invisible leader path (no line \
                 is drawn); creation supports the straight and spline path types"
                    .into(),
            )),
        };
    let max_points = MAX_CREATED_LEADER_POINTS;
    check_leader_points(max_points)?;
    Ok(LeaderOptions {
        path,
        max_points,
        arrowhead: true,
    })
}

/// Refuse a vertex count above the creation limit.
///
/// Split from [`plan_leader_options`] so the limit has one named refusal that a
/// future multi-vertex payload can reuse without re-deriving the rule.
pub(crate) fn check_leader_points(requested: usize) -> CadResult<()> {
    if requested > MAX_CREATED_LEADER_POINTS {
        return Err(CadError::Unsupported(format!(
            "mleader.option_max_points: a created leader has {MAX_CREATED_LEADER_POINTS} vertices \
             (the captured arrow and the style-derived dogleg start); {requested} were requested \
             and the payload has no vertex list, so nothing was written"
        )));
    }
    Ok(())
}

/// The `mleader.options` report row: the option set the created leader honours.
pub(crate) fn options_report(options: LeaderOptions) -> Diagnostic {
    let path = match options.path {
        MLeaderPathType::StraightLineSegments => "straight-line",
        MLeaderPathType::Spline => "spline",
        MLeaderPathType::Invisible => "invisible",
    };
    Diagnostic {
        object: None,
        code: "mleader.options".into(),
        message: format!(
            "MLEADER options: leader type {path} (from the MLEADERSTYLE path type), annotation \
             type text (style content kind MText), {} point(s) (creation derives the captured \
             arrow plus the style dogleg start), arrowhead on (style size); QLEADER and LEADER \
             share this one plan",
            options.max_points
        ),
    }
}

/// Refuse an annotation type the model cannot create.
///
/// Exposed for the same reason as [`check_leader_points`]: the content-kind rule
/// has one named implementation, and the typed grammar cannot ask for a second
/// annotation type today.
pub(crate) fn check_annotation_type(content: MLeaderContent) -> CadResult<()> {
    if content != MLeaderContent::Text {
        return Err(CadError::Unsupported(
            "mleader.content_unsupported: only MText annotation content is created".into(),
        ));
    }
    Ok(())
}

impl Application {
    /// `QLEADER` / `LEADER`: create one MULTILEADER from the typed arrow,
    /// landing and text.
    pub(crate) fn leader_command(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        id: CommandId,
    ) -> CadResult<CommandOutcome> {
        let prefix = if id == CommandId::QLeader {
            "qleader"
        } else {
            "leader"
        };
        let request = match &command.payload {
            CommandPayload::Annotation(payload) => match payload.as_ref() {
                AnnotationBatchPayload::Leader(request) => request.clone(),
                _ => {
                    return Err(invalid(
                        &format!("{prefix}.payload"),
                        format!(
                            "{prefix} takes `<arrow x,y> <landing x,y> <text>` (the selection is \
                             not its input)"
                        ),
                    ))
                }
            },
            CommandPayload::None => {
                return Err(invalid(
                    &format!("{prefix}.needs_arguments"),
                    format!(
                        "{prefix} needs `<arrow x,y> <landing x,y> <text>`; a bare selector starts \
                         no capture in this build"
                    ),
                ))
            }
            _ => {
                return Err(invalid(
                    &format!("{prefix}.payload"),
                    format!(
                        "{prefix} takes `<arrow x,y> <landing x,y> <text>`; the selection is not \
                         its input"
                    ),
                ))
            }
        };
        if !is_finite_point(request.arrow) || !is_finite_point(request.landing) {
            return Err(invalid(
                &format!("{prefix}.invalid_point"),
                "both the arrow and the landing point must be finite",
            ));
        }
        // Reuse the one MLEADER creation path by rebuilding its ordinary
        // payload; `create_mleader` owns every style/geometry/text/option refusal
        // and the single transaction.
        let mleader = Command {
            schema_version: command.schema_version,
            id: CommandId::MLeader,
            document: command.document,
            viewport: command.viewport,
            payload: CommandPayload::MLeader {
                arrow: request.arrow,
                landing: request.landing,
                text: request.text,
            },
        };
        self.create_mleader(session, &mleader)
    }
}
