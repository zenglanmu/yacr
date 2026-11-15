//! Wave 2 **named refusals** for the extended command batch.
//!
//! Five AutoCAD 2026 names describe features this build has no model or API
//! for: hyperlinks, input-search settings, digital signatures and plot-job
//! details (plus their command-line spellings). `BASE` and `LIMITS` graduated on
//! 2026-11-16 to real vector-header writes and are no longer refusals.
//!
//! Each name is admitted by the vocabulary and then refused with
//! `CadError::Unsupported` carrying a **stable** reason string (the same shape
//! `LAYOUTMERGE`/`DIMBREAK` use). That is a legitimate partial implementation:
//! the command path exists and answers honestly, but it must never be documented
//! as an implemented feature. A refusal writes nothing, bumps no revision and
//! records no `cad-history` entry, so every one of these is Viewer-legal.
//!
//! A caller payload is refused first with `require_none_payload`, so a malformed
//! request never reaches the feature-specific reason.
//!
//! # Graduated writes (2026-11-11)
//!
//! `VPLAYER`, `MVSETUP`, `PSBSCALE` and `PSTSCALE` were Wave 2 refusals and are
//! now real 2D paper-space features (per-viewport layer visibility, a scaled
//! viewport grid, and the two paper-space scale writes). They are dispatched
//! through [`Application::extended_paper_write`] to
//! [`crate::app_layout::paper_writes`]; the five names left here keep their
//! exact published reason strings.
//!
//! # Graduated vector-header writes (2026-11-16)
//!
//! `BASE` (`$INSBASE`) and `LIMITS` (`$LIMMIN`/`$LIMMAX`) were part of this
//! refusal table because the database carried no drawing-header model for them.
//! `cad-db` now carries both (`cad_db::drawing::header_geometry`), the exporter
//! writes all three header variables back, and `SETVAR` reads/writes the same
//! three names through these handlers — so both commands are real read/write
//! commands. Both ids are Work-only: each is a document mutation in AutoCAD, and
//! the read form is the report in this build (the same value `SETVAR` prints).

use super::*;
use crate::app_reports_ext::{diagnostic, report_outcome};
use crate::app_utility::{document_drawing, require_none_payload};

impl Application {
    /// Dispatch one Wave 2 command: the two graduated **vector-header writes**
    /// (`BASE`, `LIMITS`) or a stable named refusal for the five names left.
    ///
    /// `BASE` and `LIMITS` graduated on 2026-11-16: `cad-db` now carries
    /// `$INSBASE`, `$LIMMIN` and `$LIMMAX` (`drawing::header_geometry`) and the
    /// exporter writes all three, so the two commands are real read/write
    /// commands instead of refusals. Their reason strings are gone on purpose —
    /// a name that now works must not keep advertising `*.unsupported`.
    ///
    /// The five names that stay refusals keep their exact published reason
    /// strings, refuse a caller payload first (`require_none_payload`) and write
    /// nothing, bump no revision and record no history.
    pub(crate) fn extended_refusal(
        &mut self,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::BasePoint => self.insertion_base(command),
            ExtendedCommandId::DrawingLimits => self.drawing_limits(command),
            other => {
                let name = other.selector();
                require_none_payload(command, name)?;
                Err(CadError::Unsupported(reason(other)))
            }
        }
    }

    /// `BASE [<x> <y> [<z>]]`: report or write the model-space insertion base
    /// point (`$INSBASE`).
    ///
    /// The bare form is a read-only report. `BASE <x> <y> [<z>]` writes the point
    /// as one `cad-db` mutation: one revision, one `ChangeSet`, no entity change
    /// and — like every header write in this build — **no undo step** (declared,
    /// see [`crate::app_extended_refusals`] and `cad_db::drawing::header_geometry`).
    /// The value is stored raw and the exporter writes it back as `$INSBASE`.
    ///
    /// A payload that is not one finite point is refused by name; the drawing is
    /// untouched.
    fn insertion_base(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let drawing = document_drawing(self, command)?.clone();
        let current = drawing.insertion_base();
        let point = match &command.payload {
            CommandPayload::None => {
                return Ok(report_outcome(vec![match current {
                    Some(base) => diagnostic(
                        "base.value",
                        format!(
                            "BASE: the drawing's insertion base point ($INSBASE) is {}, {}, {}; set \
                             it with `BASE <x> <y> [<z>]`",
                            base.x, base.y, base.z
                        ),
                    ),
                    None => diagnostic(
                        "base.not_stored",
                        "BASE: this drawing stores no insertion base point ($INSBASE). The \
                         importer of this build does not read $INSBASE from a source file, so no \
                         origin value is reported as if it were the file's base point; set it with \
                         `BASE <x> <y> [<z>]`"
                            .into(),
                    ),
                }]));
            }
            CommandPayload::Points(points) => match points.as_slice() {
                [point] => *point,
                _ => {
                    return Err(CadError::InvalidInput(format!(
                        "base.invalid_point: BASE takes one point (`BASE <x> <y> [<z>]`), got {} \
                         point(s); nothing was written",
                        points.len()
                    )));
                }
            },
            _ => {
                return Err(CadError::InvalidInput(
                    "base.invalid_payload: BASE takes no payload (report) or one point".into(),
                ));
            }
        };
        let transaction = crate::app_drawing::next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let changeset =
            candidate.set_insertion_base(point, transaction, "set insertion base point")?;
        let changed = changeset.before != changeset.after;
        if changed {
            self.workspace
                .documents
                .get_mut(&document_id)
                .ok_or_else(|| CadError::Invariant("document disappeared while publishing".into()))?
                .drawing = Arc::new(candidate);
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(changeset)),
            diagnostics: vec![
                diagnostic(
                    "base.applied",
                    format!(
                        "BASE: insertion base point ($INSBASE) is now {}, {}, {}{}",
                        point.x,
                        point.y,
                        point.z,
                        if changed {
                            ""
                        } else {
                            " (already stored, no change)"
                        }
                    ),
                ),
                diagnostic(
                    "base.no_history",
                    "BASE: this write published one revision and one ChangeSet and recorded NO undo \
                     step; cad-history has no header-variable patch kind"
                        .into(),
                ),
            ],
            measurement: None,
            point_report: None,
        })
    }

    /// `LIMITS <min_x> <min_y> <max_x> <max_y>`: report or write the model-space
    /// drawing limits (`$LIMMIN` / `$LIMMAX`).
    ///
    /// The bare form reports both stored corners (or names the ones this drawing
    /// does not store). The typed form writes both corners as **one** `cad-db`
    /// mutation (one revision, one `ChangeSet`, no undo step). `LIMITS ON`/`OFF`
    /// (`LIMCHECK` enforcement) is a named refusal: this build has no
    /// limit-checking model, and switching it on would promise a clamp that no
    /// path performs.
    fn drawing_limits(&mut self, command: &Command) -> CadResult<CommandOutcome> {
        let document_id = command.document;
        let drawing = document_drawing(self, command)?.clone();
        let corners = match &command.payload {
            CommandPayload::None => {
                let row = |code: &str, message: String| diagnostic(code, message);
                let mut rows = Vec::with_capacity(3);
                match (drawing.limits_min(), drawing.limits_max()) {
                    (Some(min), Some(max)) => rows.push(row(
                        "limits.value",
                        format!(
                            "LIMITS: model-space limits ($LIMMIN/$LIMMAX) are {} {} to {} {}; set \
                             them with `LIMITS <min_x> <min_y> <max_x> <max_y>`",
                            min[0], min[1], max[0], max[1]
                        ),
                    )),
                    (min, max) => rows.push(row(
                        "limits.incomplete",
                        format!(
                            "LIMITS: this drawing stores {} ({}); set both corners with `LIMITS \
                             <min_x> <min_y> <max_x> <max_y>`",
                            if min.is_some() {
                                "only $LIMMIN"
                            } else if max.is_some() {
                                "only $LIMMAX"
                            } else {
                                "no limit corner"
                            },
                            match (min, max) {
                                (Some(min), _) => format!("$LIMMIN = {} {}", min[0], min[1]),
                                (_, Some(max)) => format!("$LIMMAX = {} {}", max[0], max[1]),
                                _ =>
                                    "the importer of this build does not read $LIMMIN/$LIMMAX from \
                                      a source file and no command has written them"
                                        .to_string(),
                            }
                        ),
                    )),
                }
                rows.push(row(
                    "limits.check_unsupported",
                    "LIMITS: `LIMITS ON|OFF` (LIMCHECK drawing-limit enforcement) is NOT \
                     IMPLEMENTED — this build models no limit checking, so no point is clamped and \
                     no grid is limited; only the stored $LIMMIN/$LIMMAX values are written"
                        .into(),
                ));
                return Ok(report_outcome(rows));
            }
            CommandPayload::Points(points) => match points.as_slice() {
                [min, max] => (*min, *max),
                _ => {
                    return Err(CadError::InvalidInput(format!(
                        "limits.invalid_corners: LIMITS takes two corners (`LIMITS <min_x> <min_y> \
                         <max_x> <max_y>`), got {} point(s); nothing was written",
                        points.len()
                    )));
                }
            },
            _ => {
                return Err(CadError::InvalidInput(
                    "limits.invalid_payload: LIMITS takes no payload (report) or two corners"
                        .into(),
                ));
            }
        };
        // The corners are 2D header values; a z value would be silently lost, so
        // a non-zero z is refused instead of ignored.
        for (corner, label) in [(corners.0, "minimum"), (corners.1, "maximum")] {
            if corner.z != 0.0 {
                return Err(CadError::InvalidInput(format!(
                    "limits.non_planar: the {label} limit corner has z = {}; $LIMMIN/$LIMMAX are 2D \
                     values and nothing was written",
                    corner.z
                )));
            }
        }
        let transaction = crate::app_drawing::next_transaction_id();
        let mut candidate = drawing.as_ref().clone();
        let changeset = candidate.set_model_limits(
            Some([corners.0.x, corners.0.y]),
            Some([corners.1.x, corners.1.y]),
            transaction,
            "set drawing limits",
        )?;
        let changed = changeset.before != changeset.after;
        if changed {
            self.workspace
                .documents
                .get_mut(&document_id)
                .ok_or_else(|| CadError::Invariant("document disappeared while publishing".into()))?
                .drawing = Arc::new(candidate);
        }
        Ok(CommandOutcome {
            objects: Vec::new(),
            changes: Some(Arc::new(changeset)),
            diagnostics: vec![
                diagnostic(
                    "limits.applied",
                    format!(
                        "LIMITS: model-space limits are now {} {} to {} {}{}",
                        corners.0.x,
                        corners.0.y,
                        corners.1.x,
                        corners.1.y,
                        if changed {
                            ""
                        } else {
                            " (already stored, no change)"
                        }
                    ),
                ),
                diagnostic(
                    "limits.no_history",
                    "LIMITS: this write published one revision and one ChangeSet and recorded NO \
                     undo step; cad-history has no header-variable patch kind"
                        .into(),
                ),
                diagnostic(
                    "limits.check_unsupported",
                    "LIMITS: the values are stored and exported, but this build enforces no \
                     drawing-limit checking (LIMCHECK); no point is clamped"
                        .into(),
                ),
            ],
            measurement: None,
            point_report: None,
        })
    }

    /// Dispatch one graduated Wave 2 **paper-space write**.
    ///
    /// Each arm is a real write (one transaction / one layout patch) except the
    /// bare `VPLAYER` report, which is read-only. The module that owns the
    /// implementation documents every `*.` refusal code.
    pub(crate) fn extended_paper_write(
        &mut self,
        session: &mut SessionState,
        command: &Command,
        ext: ExtendedCommandId,
    ) -> CadResult<CommandOutcome> {
        match ext {
            ExtendedCommandId::VPlayer => self.vplayer(session, command),
            ExtendedCommandId::MvSetup => self.mv_setup(session, command),
            ExtendedCommandId::PsbScale => self.psb_scale(session, command),
            ExtendedCommandId::PstScale => self.pst_scale(session, command),
            // `CHSPACE` moves entities between model and paper space through the
            // active viewport's resolved transform (one transaction / one undo
            // step); it lives with the other paper-space writes.
            ExtendedCommandId::ChSpace => self.ch_space(session, command),
            other => Err(CadError::Unsupported(reason(other))),
        }
    }
}

/// The stable refusal reason for one Wave 2 command.
///
/// These strings are contract-tested and published in
/// `docs/autocad-2026-command-coverage.md`; changing one is a behaviour change.
fn reason(ext: ExtendedCommandId) -> String {
    let text = match ext {
        ExtendedCommandId::HyperlinkOptions => {
            "hyperlinkoptions.unsupported: this build has no hyperlink storage model"
        }
        ExtendedCommandId::InputSearchOptions => {
            "inputsearchoptions.unsupported: input-search settings are not modeled"
        }
        ExtendedCommandId::DashInputSearchOptions => {
            "-inputsearchoptions.unsupported: input-search settings are not modeled"
        }
        ExtendedCommandId::SigValidate => {
            "sigvalidate.unsupported: this build has no digital-signature model"
        }
        ExtendedCommandId::ViewPlotDetails => "viewplotdetails.unsupported: no plot-details model",
        other => {
            return format!(
                "{}: this command is not a Wave 2 refusal",
                other.selector().to_ascii_lowercase()
            );
        }
    };
    text.to_string()
}
