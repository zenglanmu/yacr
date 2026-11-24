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
//! | maximum points | line vertex list | declared limit [`MAX_CREATED_LEADER_POINTS`] (the typed line has two coordinates); a request above it is refused by name |
//! | annotation type (text) | style `170` content kind + typed text | honoured: a block/tolerance style is refused by name; the typed text decides MText content versus a content-less leader |
//! | arrowhead on/off | style `44` arrowhead size | an arrowhead-off row (`0`) is refused by name instead of silently drawing one |
//!
//! The refusal codes are the shared `mleader.*` ones: there is exactly one
//! construction point, so `QLEADER`/`LEADER` deliberately do **not** fork the
//! option code namespace (their own prefix only covers their typed-argument
//! grammar: `qleader.payload`, `qleader.invalid_point`, …).

use super::*;
use cad_db::{MLeaderStyle, MLeaderStyleResolution};
use cad_domain::{MLeaderContentKind, MLeaderPathType};

/// The number of vertices a created leader derives.
///
/// [`MultiLeaderGeometry::created`] is the single derivation: vertex 1 is the
/// captured arrow position and vertex 2 the style-derived dogleg start. The typed
/// line has exactly two coordinates, so no caller can request more; this constant
/// and [`plan_leader_options`]'s refusal exist so that the limit is **named**
/// rather than implied.
///
/// The ceiling is an **input** limit, not a model limit: DXF `LEADER_LINE` carries
/// a vertex list, the domain [`cad_domain::MLeaderLine`] stores it as a `Vec`, and
/// the importer/exporter round-trip a multi-vertex line. Only the creation
/// grammar is two-point — the AutoCAD way to get a second leader line is
/// `MLEADEREDIT ADD`, which this build implements.
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
    /// The style row's declared content kind (`170`). The created entity's own
    /// kind comes from the typed `CONTENT` option (or the typed text); the
    /// command sets the `0x400` override bit only when the two differ.
    pub(crate) style_content: MLeaderContentKind,
}

/// Resolve and validate the creation options from the drawing's style row.
///
/// `row` is the **stored** row (the raw `arrowhead_size` matters: the resolved
/// snapshot clamps a `0` to the pinned default, which would hide an
/// arrowhead-off style), `resolution` the single resolved snapshot the created
/// entity stores. `arrowhead` is the typed `QLEADER ARROW ON|OFF` request:
/// `Some(false)` creates a line with no arrowhead (size `0` plus the per-line
/// override bit), so a style that draws none is usable; every other request
/// draws the style arrowhead, so a non-positive style size is refused.
///
/// The annotation kind is **not** decided here: the typed `CONTENT` option (or,
/// without one, the typed text) decides what the created entity carries, and
/// every one of the four kinds is creatable now — a block/tolerance **style row**
/// is therefore a default, not a refusal.
///
/// Refusals (named, nothing written):
///
/// * `mleader.content_unsupported` — a style row whose content code is outside
///   the four known kinds (`0`..`3`);
/// * `mleader.option_arrowhead_off` — the row disables the arrowhead (size `0`)
///   and the creation does not explicitly ask for no arrowhead;
/// * `mleader.path_type` — an invisible leader path (no line at all);
/// * `mleader.option_max_points` — a requested vertex count above
///   [`MAX_CREATED_LEADER_POINTS`];
/// * `mleader.style_invalid` — a non-finite arrowhead size.
pub(crate) fn plan_leader_options(
    row: &MLeaderStyle,
    resolution: &MLeaderStyleResolution,
    arrowhead: Option<bool>,
) -> CadResult<LeaderOptions> {
    let style_content = MLeaderContentKind::from_dxf(row.content_type).ok_or_else(|| {
        CadError::Unsupported(format!(
            "mleader.content_unsupported: the resolved MLEADERSTYLE declares unknown content kind \
             {} (the DXF codes are 0 none, 1 block, 2 MText, 3 tolerance)",
            row.content_type
        ))
    })?;
    if !row.arrowhead_size.is_finite() {
        return Err(CadError::InvalidInput(format!(
            "mleader.style_invalid: the arrowhead size must be finite, got {}",
            row.arrowhead_size
        )));
    }
    let draw_arrowhead = arrowhead.unwrap_or(true);
    if draw_arrowhead && row.arrowhead_size <= 0.0 {
        return Err(CadError::Unsupported(format!(
            "mleader.option_arrowhead_off: the resolved MLEADERSTYLE disables the arrowhead \
             (size {}); this creation draws the style arrowhead, so nothing was written (ask for \
             `ARROW OFF` to create an arrow-less leader)",
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
        arrowhead: draw_arrowhead,
        style_content,
    })
}

/// Refuse a vertex count above the creation limit.
///
/// Split from [`plan_leader_options`] so the limit has one named refusal that a
/// future multi-vertex payload can reuse without re-deriving the rule. The reason
/// is stated in the refusal: the ceiling is the **typed input's** two coordinates,
/// not a limit of DXF `LEADER_LINE` or of the stored `MLeaderLine` (both carry a
/// real vertex list); a second leader *line* is `MLEADEREDIT ADD`'s job.
pub(crate) fn check_leader_points(requested: usize) -> CadResult<()> {
    if requested > MAX_CREATED_LEADER_POINTS {
        return Err(CadError::Unsupported(format!(
            "mleader.option_max_points: a created leader has {MAX_CREATED_LEADER_POINTS} \
             vertices because the typed creation line carries exactly two coordinates (the \
             captured arrow and the landing point) and the dogleg start is derived from them; \
             {requested} were requested. DXF LEADER_LINE and the stored MLeaderLine do carry a \
             vertex list (import and export round-trip a multi-vertex line) and additional \
             leader lines come from MLEADEREDIT ADD, so this is an input limit, not a model \
             limit — nothing was written"
        )));
    }
    Ok(())
}

/// The `mleader.options` report row: the option set the created leader honours.
pub(crate) fn options_report(options: LeaderOptions, content: MLeaderContentKind) -> Diagnostic {
    let path = match options.path {
        MLeaderPathType::StraightLineSegments => "straight-line",
        MLeaderPathType::Spline => "spline",
        MLeaderPathType::Invisible => "invisible",
    };
    let annotation = match content {
        MLeaderContentKind::Text => "text (MText)",
        MLeaderContentKind::None => "none (content-less: no annotation, arrow and dogleg only)",
        MLeaderContentKind::Block => "block (an inserted block reference)",
        MLeaderContentKind::Tolerance => "tolerance (a feature-control frame)",
    };
    let arrowhead = if options.arrowhead {
        "on (line LEADER_LINE 40 = the style size)"
    } else {
        "off (line LEADER_LINE 40 = 0 with the per-line override bit 16)"
    };
    Diagnostic {
        object: None,
        code: "mleader.options".into(),
        message: format!(
            "MLEADER options: leader type {path} (from the MLEADERSTYLE path type), annotation \
             type {annotation} (entity 172, style kind {}; the entity value wins with override \
             bit 0x400 when the two differ), {} point(s) (creation derives the captured arrow \
             plus the style dogleg start; the typed line carries exactly two coordinates, more \
             leader lines come from MLEADEREDIT ADD and multi-vertex lines only arrive by \
             import), arrowhead {arrowhead}; QLEADER and LEADER share this one plan",
            options.style_content.dxf(),
            options.max_points
        ),
    }
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
        // `QLEADER MAXPOINTS` is an input bound, not a geometry field: the
        // creation derives exactly two vertices, so anything smaller than two is
        // unusable and anything larger is the shared `mleader.option_max_points`
        // refusal (raised by `create_mleader_with`). Both happen before any work.
        if let Some(requested) = request.max_points {
            if requested < 2 {
                return Err(invalid(
                    &format!("{prefix}.invalid_max_points"),
                    format!(
                        "MAXPOINTS must be at least 2 (the created leader's arrow and dogleg \
                         start), got {requested}; nothing was written"
                    ),
                ));
            }
        }
        let arrowhead = request.arrowhead;
        let max_points = request.max_points;
        let content = request.content.clone();
        // Reuse the one MLEADER creation path by rebuilding its ordinary
        // payload; `create_mleader_with` owns every style/geometry/text/option
        // refusal and the single transaction. The legacy `CONTENT`/`ARROW`/
        // `MAXPOINTS` ride beside it as the same typed values `MLEADER` uses.
        let mleader = Command {
            schema_version: command.schema_version,
            id: CommandId::MLeader,
            document: command.document,
            viewport: command.viewport,
            payload: CommandPayload::MLeader {
                arrow: request.arrow,
                landing: request.landing,
                text: request.text,
                // The style is the resolved session/`Standard` row: neither
                // legacy command has a typed `STYLE` option.
                style_name: None,
                dogleg_length: None,
                dogleg_angle: None,
                attachment: None,
                text_angle: None,
                // The content kind is the shared `CONTENT` request, passed to the
                // creation path below (never silently dropped).
                content: None,
            },
        };
        self.create_mleader_with(session, &mleader, content, arrowhead, max_points)
    }
}
