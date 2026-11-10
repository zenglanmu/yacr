//! The typed `PDMODE <n>` / `PDSIZE <n>` lines: the drawing POINT display style
//! sysvars.
//!
//! Both are **document writes**, unlike the session-scoped `SNAPUNIT`: the value
//! goes into the open drawing's header (`$PDMODE` / `$PDSIZE`) through the
//! ordinary command funnel, so it bumps the document revision (which is what
//! refreshes the cached marker style) and records **no** undo entry (`cad-history`
//! has no header patch kind). The command layer owns the validation; this module
//! only parses the typed argument with the one grammar in
//! [`super::parse_pdmode_argument`] / [`super::parse_pdsize_argument`] and
//! dispatches the corresponding payload.
//!
//! `PDMODE` is an **integer** field: a fractional `PDMODE 3.5` is refused with a
//! stable reason rather than rounded, because `$PDMODE` is a bit field and
//! rounding would draw a marker nobody asked for.
//!
//! Kept in its own module only to keep `session/mod.rs` within the file-size
//! budget; the argument-selector dispatch
//! ([`CadSession::argument_command`]) lives here for the same reason.

use cad_app::{CommandId, CommandPayload};

use super::annot_arguments::{
    GROUPEDIT_SELECTOR, GROUP_SELECTOR, HATCHSETORIGIN_SELECTOR, LEADER_SELECTOR, QLEADER_SELECTOR,
    TEXTFIT_SELECTOR, UNGROUP_SELECTOR,
};
use super::commands::{
    parse_linetype_argument, parse_mocoro_argument, parse_pan_argument, parse_trace_argument,
    parse_viewres_argument, parse_vscurrent_argument, ARRAYPATH_SELECTOR, AUDIT_SELECTOR,
    CHAMFER_SELECTOR, DASH_TABLE_SELECTOR, DONUT_SELECTOR, DRAWORDER_SELECTOR, FILLET_SELECTOR,
    FILTER_SELECTOR, FIND_REPORT_SELECTOR, FIND_SELECTOR, GETSEL_SELECTOR, HELIX_SELECTOR,
    IMAGEATTACH_SELECTOR, IMAGECLIP_SELECTOR, JOIN_SELECTOR, LINETYPE_SELECTOR, MIRROR_SELECTOR,
    MLINE_SELECTOR, MLSTYLE_SELECTOR, MOCORO_SELECTOR, MPEDIT_SELECTOR, MREDO_SELECTOR,
    MULTIPLE_SELECTOR, OFFSET_SELECTOR, OVERKILL_SELECTOR, PAN_SELECTOR, PURGE_SELECTOR,
    QSELECT_SELECTOR, RENAME_SELECTOR, REVCLOUD_SELECTOR, SETVAR_SELECTOR, SOLID_SELECTOR,
    SSX_SELECTOR, TABLEDIT_SELECTOR, TABLE_SELECTOR, TEXTMASK_SELECTOR, TRACE_SELECTOR,
    TREX_SELECTOR, TXT2MTXT_SELECTOR, VIEWRES_SELECTOR, VSCURRENT_SELECTOR, WIPEOUT_SELECTOR,
};
use super::diagnostics::cad_error_text;
use super::dim_arguments::{
    DIMEDIT_SELECTOR, DIMJOGLINE_SELECTOR, DIMOVERRIDE_SELECTOR, DIMSPACE_SELECTOR,
    DIMSTYLE_SELECTOR, DIMTEDIT_SELECTOR, QDIM_SELECTOR,
};
use super::{
    CadSession, ARGUMENT_SELECTOR, HATCHEDIT_SELECTOR, JUSTIFYTEXT_SELECTOR, PDMODE_SELECTOR,
    PDSIZE_SELECTOR, PEDIT_SELECTOR, SNAPUNIT_SELECTOR, SPACE_SELECTOR, TCASE_SELECTOR,
    TCIRCLE_SELECTOR, TCOUNT_SELECTOR, TEXTALIGN_SELECTOR, TORIENT_SELECTOR, TSCALE_SELECTOR,
};

impl CadSession {
    /// Dispatch one typed `SELECTOR <argument>` line to its own command path.
    ///
    /// The splitter ([`super::typed_command_argument`]) has already admitted the
    /// base selector and kept the argument text verbatim, so this is the single
    /// place that decides which path an argument-taking selector takes. An
    /// unlisted selector keeps the `ZOOM` path, which is the only remaining
    /// argument-taking selector by contract (`ARGUMENT_SELECTORS`).
    pub(super) fn argument_command(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            SNAPUNIT_SELECTOR => self.snap_unit_argument(argument, text),
            PDMODE_SELECTOR | PDSIZE_SELECTOR => {
                self.point_style_argument(selector, argument, text)
            }
            // `SPACE <argument>`: a tab click is one ordinary typed line, not a
            // new IPC. Bare `SPACE` never reaches here (it keeps the name-only
            // payload refusal).
            SPACE_SELECTOR => self.space_argument(argument),
            // `TORIENT <angle>` / `TCASE <mode>` (2026-11-07): the selection is
            // still the input; only the rotation/mode rides the payload.
            TORIENT_SELECTOR | TCASE_SELECTOR => {
                self.text_command_argument(selector, argument, text)
            }
            // `PEDIT <option>` (2026-11-07): the one selected polyline is the
            // input; the option travels in the payload.
            PEDIT_SELECTOR => self.pedit_argument(argument, text),
            // The interactive modify option lines (2026-11-09): JOIN's modes,
            // the OFFSET/MIRROR stored options, and the CHAMFER/FILLET stored
            // modes plus their polyline forms.
            JOIN_SELECTOR | OFFSET_SELECTOR | MIRROR_SELECTOR | CHAMFER_SELECTOR
            | FILLET_SELECTOR => self.modify_argument(selector, argument, text),
            // `HATCHEDIT <option>` (2026-11-07): the one selected hatch is the
            // input; the typed option travels in the payload.
            HATCHEDIT_SELECTOR => self.hatchedit_argument(argument, text),
            // `MEASUREGEOM <option>` routes to the existing measurement tools
            // (`DISTANCE`/`ANGLE`/`AREA`/`POLYLINE`) or the report-only `RADIUS`
            // (and the explicit `VOLUME` refusal). It is not a second engine.
            super::commands::MEASUREGEOM_SELECTOR => self.measuregeom_argument(argument, text),
            // The 2026-11-07 2D text batch: `<H> <V>` / factor / edge / run /
            // shape ride the payload; the selection is still the input.
            JUSTIFYTEXT_SELECTOR | TSCALE_SELECTOR | TEXTALIGN_SELECTOR | TCOUNT_SELECTOR
            | TCIRCLE_SELECTOR => self.text_batch_argument(selector, argument, text),
            // The 2026-11-07 layer batch argument forms.
            super::commands::LAYERSTATE_SELECTOR => self.layer_state_argument(argument, text),
            super::commands::LAYWALK_SELECTOR => self.layer_walk_argument(argument, text),
            super::commands::LTSCALE_SELECTOR => self.linetype_scale_argument(argument, text),
            super::commands::LAYERPMODE_SELECTOR => self.layer_pmode_argument(argument, text),
            // The 2026-11-07 selection/filter batch: all five are typed argument
            // forms of canonical selectors, never new commands.
            QSELECT_SELECTOR | FILTER_SELECTOR | GETSEL_SELECTOR => {
                self.select_filter_argument(selector, argument, text)
            }
            SSX_SELECTOR => self.ssx_argument(argument, text),
            AUDIT_SELECTOR => self.audit_argument(argument, text),
            FIND_SELECTOR | FIND_REPORT_SELECTOR => self.find_argument(selector, argument, text),
            // The 2026-11-07 utility/named-object batch.
            PURGE_SELECTOR | RENAME_SELECTOR | SETVAR_SELECTOR | MREDO_SELECTOR
            | MULTIPLE_SELECTOR => self.utility_argument(selector, argument, text),
            // The 2026-11-07 draw/modify batch typed forms.
            DONUT_SELECTOR | SOLID_SELECTOR | HELIX_SELECTOR | MLINE_SELECTOR
            | MLSTYLE_SELECTOR | TREX_SELECTOR | MPEDIT_SELECTOR | DRAWORDER_SELECTOR => {
                self.draw2_argument(selector, argument, text)
            }
            // `ARRAYPATH <count> [ALIGN]`: count/align ride the payload; the
            // selection is the whole input.
            ARRAYPATH_SELECTOR => self.arraypath_argument(argument, text),
            // The raster-image typed forms: the file/pixel/scale or clip rectangle
            // ride the payload; the command layer owns every semantic refusal.
            IMAGEATTACH_SELECTOR | IMAGECLIP_SELECTOR => {
                self.image_argument(selector, argument, text)
            }
            // The table typed forms: `TABLE`/`-TABLE` creation and `TABLEDIT`.
            TABLE_SELECTOR | DASH_TABLE_SELECTOR | TABLEDIT_SELECTOR => {
                self.table_argument(selector, argument, text)
            }
            // The 2026-11-07 view batch: named views and the camera direction.
            // `-VIEW` normalises to `VIEW` before this dispatch, so it lands on
            // the same arm.
            super::commands::VIEW_SELECTOR => self.view_state_argument(argument, text),
            super::commands::VIEWGO_SELECTOR => self.view_go_argument(argument, text),
            super::commands::VPOINT_SELECTOR => self.vpoint_argument(argument, text),
            // The 2026-11-07 layout/viewport batch typed forms, including the
            // real `PAGESETUP`/`PSETUPIN` page-setup writes. Each dash spelling
            // is its own extended selector and shares this grammar.
            super::commands::LAYOUT_SELECTOR
            | super::commands::LAYOUTMERGE_SELECTOR
            | super::commands::LAYOUTWIZARD_SELECTOR
            | super::commands::PAGESETUP_SELECTOR
            | super::commands::DASH_PAGESETUP_SELECTOR
            | super::commands::PSETUPIN_SELECTOR
            | super::commands::DASH_PSETUPIN_SELECTOR
            | super::commands::VPORTS_SELECTOR
            | super::commands::VPCLIP_SELECTOR
            | super::commands::VPSCALE_SELECTOR
            | super::commands::LAYVPI_SELECTOR => self.layout_argument(selector, argument, text),
            // `MVIEW <minx> <miny> <maxx> <maxy>` carries a two-corner point
            // payload, so it takes its own typed path.
            super::commands::MVIEW_SELECTOR => self.mview_argument(argument, text),

            // `DIMSPACE <spacing>`: the explicit positive row pitch rides the
            // payload; a bare `DIMSPACE` keeps the name-only Auto form.
            DIMSPACE_SELECTOR => self.dimspace_argument(argument, text),

            // The 2026-11-07 dimension-editing batch typed forms.
            DIMSTYLE_SELECTOR | DIMTEDIT_SELECTOR | DIMEDIT_SELECTOR | DIMOVERRIDE_SELECTOR
            | DIMJOGLINE_SELECTOR | QDIM_SELECTOR => self.dim_argument(selector, argument, text),

            // The 2026-11-07 annotation/group batch typed forms.
            GROUP_SELECTOR
            | UNGROUP_SELECTOR
            | GROUPEDIT_SELECTOR
            | QLEADER_SELECTOR
            | LEADER_SELECTOR
            | TEXTFIT_SELECTOR
            | HATCHSETORIGIN_SELECTOR => self.annot_argument(selector, argument, text),
            // The 2026-11-07 extras batch typed forms (`-PAN` normalizes to
            // `PAN` before this dispatch, `-LINETYPE` to `LINETYPE`).
            PAN_SELECTOR | TRACE_SELECTOR | MOCORO_SELECTOR | VIEWRES_SELECTOR
            | VSCURRENT_SELECTOR | LINETYPE_SELECTOR => {
                self.extras_argument(selector, argument, text)
            }
            // The 2026-11 P3 option batch: the payloads already exist, only the
            // typed argument grammar/dispatch was missing.
            OVERKILL_SELECTOR | REVCLOUD_SELECTOR | WIPEOUT_SELECTOR | TEXTMASK_SELECTOR
            | TXT2MTXT_SELECTOR => self.payload_option_argument(selector, argument, text),

            _ => {
                debug_assert_eq!(selector, ARGUMENT_SELECTOR);
                self.typed_argument_command(selector, argument, text)
            }
        }
    }

    /// Apply one typed `PDMODE <n>` / `PDSIZE <n>` line.
    ///
    /// | argument | behaviour |
    /// | --- | --- |
    /// | `PDMODE` whole number in `i16` (`0`, `3`, `35`, `-1`) | store the **raw** `$PDMODE` in the drawing header |
    /// | `PDSIZE` finite decimal (`2.5`, `0`, `-20`) | store the **raw** `$PDSIZE` in the drawing header (`> 0` absolute, `0` viewport-relative, `< 0` percent of the viewport height) |
    /// | anything else (`PDMODE 3.5`, `PDMODE 40000`, `PDMODE abc`, `PDSIZE NaN`, `PDSIZE 2 3`) | the stable [`super::invalid_pdmode_argument_reason`] / [`super::invalid_pdsize_argument_reason`], recorded in [`CadSession`]'s error; nothing changes |
    ///
    /// A bare `PDMODE`/`PDSIZE` never reaches this method: it has no argument, so
    /// it keeps the name-only payload refusal of the application layer (an
    /// explicit "needs one numeric argument" error, never a fabricated default).
    fn point_style_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let payload = if selector == PDMODE_SELECTOR {
            match super::parse_pdmode_argument(argument) {
                Some(mode) => CommandPayload::PointDisplayMode(mode),
                None => return self.refuse_argument(super::invalid_pdmode_argument_reason(text)),
            }
        } else {
            match super::parse_pdsize_argument(argument) {
                Some(size) => CommandPayload::PointDisplaySize(size),
                None => return self.refuse_argument(super::invalid_pdsize_argument_reason(text)),
            }
        };
        let id = if selector == PDMODE_SELECTOR {
            CommandId::SetPointDisplayMode
        } else {
            CommandId::SetPointDisplaySize
        };
        // Echo the admitted selector name (never the typed value), exactly like
        // the name-only path.
        self.core.record_command(selector);
        match self.core.command(id, payload) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }

    /// Apply one typed line of the 2026-11-07 extras batch.
    ///
    /// The parser is the authority for syntax and range; the application layer
    /// re-validates every payload it receives. A refusal is recorded in
    /// [`CadSession`]'s error and changes nothing.
    fn extras_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let (id, payload) = match selector {
            PAN_SELECTOR => (
                CommandId::Pan,
                parse_pan_argument(argument)
                    .ok_or_else(|| invalid_extras_argument_reason(selector, text)),
            ),
            TRACE_SELECTOR => (
                CommandId::Trace,
                parse_trace_argument(argument)
                    .ok_or_else(|| invalid_extras_argument_reason(selector, text)),
            ),
            MOCORO_SELECTOR => (CommandId::Mocoro, parse_mocoro_argument(argument)),
            VIEWRES_SELECTOR => (CommandId::ViewRes, parse_viewres_argument(argument)),
            VSCURRENT_SELECTOR => (CommandId::VisualStyle, parse_vscurrent_argument(argument)),
            _ => {
                debug_assert_eq!(selector, LINETYPE_SELECTOR);
                (CommandId::Linetype, parse_linetype_argument(argument))
            }
        };
        let payload = match payload {
            Ok(payload) => payload,
            Err(reason) => return self.refuse_argument(reason),
        };
        self.core.record_command(selector);
        match self.core.command(id, payload) {
            Ok(()) => {
                *self.error.borrow_mut() = None;
                Ok(())
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }

    /// Record a stable argument refusal and return it (nothing changes).
    ///
    /// Shared by every typed-argument selector module (`point_style`, `space`).
    pub(super) fn refuse_argument(&self, reason: String) -> Result<(), String> {
        *self.error.borrow_mut() = Some(reason.clone());
        Err(reason)
    }
}

/// The stable refusal for an argument outside an extras selector's grammar.
fn invalid_extras_argument_reason(selector: &str, text: &str) -> String {
    format!(
        "unsupported or malformed {selector} argument in `{}`: the typed form is documented in \
         the command contract; nothing was changed",
        text.trim()
    )
}
