//! The typed-argument selector table, split out of `commands.rs` so that
//! file stays under the 1000-line code gate.

use super::*;

/// The selectors that accept a **typed argument** on the command line.
///
/// An argument is admitted by its base selector and then validated against its
/// own grammar; it is **not** a new [`CommandId`] and there is no second IPC.
/// Every other selector stays name-only, so `LINE 2` is the ordinary
/// unsupported-name refusal.
///
/// * [`ARGUMENT_SELECTOR`] (`ZOOM`): a positive factor, `nX`, `nXP` or
///   `ALL`/`EXTENTS`/`WINDOW`/`PREVIOUS`/`OBJECT` ([`parse_zoom_argument`]);
/// * [`SNAPUNIT_SELECTOR`] / [`PDMODE_SELECTOR`] / [`PDSIZE_SELECTOR`]: one
///   positive spacing / one integer / one signed decimal, in that order;
/// * [`SPACE_SELECTOR`] (`SPACE`): `MODEL`, `#<u128>` or a layout display name,
///   resolved against the authoritative drawing by the session;
/// * [`TORIENT_SELECTOR`] (`TORIENT`): one finite angle in degrees or
///   `REVERSE`/`R` ([`parse_torient_argument`]);
/// * [`TCASE_SELECTOR`] (`TCASE`): one case mode ([`parse_tcase_argument`]);
/// * [`DIMSPACE_SELECTOR`] (`DIMSPACE`): one non-negative finite spacing (`0`
///   aligns) or Auto ([`super::dim_arguments::parse_dimspace_argument`]).
///
/// PEDIT/HATCHEDIT/MEASUREGEOM take one declared option; the text batch takes
/// `<H> <V>`, a factor, an edge word, a numbering run or a shape; the layer
/// batch takes its layer-state/laywalk/ltscale/layerpmode grammars, `LAYFRZ`'s
/// `VP [<index>,...]` viewport scope ([`parse_layfrz_argument`]) and
/// `-LAYER`'s write options ([`parse_dash_layer_argument`]); a bare selector is
/// refused by name, never defaulted.
pub const ARGUMENT_SELECTORS: &[&str] = &[
    ARGUMENT_SELECTOR,
    SNAPUNIT_SELECTOR,
    PDMODE_SELECTOR,
    PDSIZE_SELECTOR,
    SPACE_SELECTOR,
    TORIENT_SELECTOR,
    TCASE_SELECTOR,
    PEDIT_SELECTOR,
    // The interactive modify option lines (2026-11-09): JOIN's two modes, the
    // OFFSET option set, MIRROR's `MIRRTEXT`/erase answers, and the CHAMFER /
    // FILLET stored modes (plus their polyline forms).
    JOIN_SELECTOR,
    OFFSET_SELECTOR,
    MIRROR_SELECTOR,
    CHAMFER_SELECTOR,
    FILLET_SELECTOR,
    HATCHEDIT_SELECTOR,
    // `HATCH` / `-HATCH`'s typed **pattern-name** forms (`PATTERN <name>`,
    // `PATTERNFILE <path.pat> [<name>]`). The selection stays the boundary; the
    // name rides `CommandPayload::HatchPattern`, because the `hatchPattern`
    // channel is a closed 1-based index and cannot carry a name.
    crate::session::HATCH_SELECTOR,
    crate::session::DASH_HATCH_SELECTOR,
    // `ARRAY` / `-ARRAY` / `ARRAYCLASSIC`'s typed `RECT`/`POLAR` forms: the
    // counts/spacings are the session channel the same planners already read, so
    // the typed line writes that one channel and dispatches the ordinary
    // `ArrayRect`/`ArrayPolar` payload.
    ARRAY_SELECTOR,
    DASH_ARRAY_SELECTOR,
    ARRAYCLASSIC_SELECTOR,
    MEASUREGEOM_SELECTOR,
    JUSTIFYTEXT_SELECTOR,
    TSCALE_SELECTOR,
    TEXTALIGN_SELECTOR,
    TCOUNT_SELECTOR,
    TCIRCLE_SELECTOR,
    LAYERSTATE_SELECTOR,
    LAYWALK_SELECTOR,
    LTSCALE_SELECTOR,
    LAYERPMODE_SELECTOR,
    // `-LAYER`'s write forms; the bare selector and `?` stay the read-only
    // report of the extended `DashLayer` command.
    DASH_LAYER_SELECTOR,
    // `-PARAMETERS`: the command-line Parameter Manager. The bare name is the
    // read-only list; the typed forms are the one-option write grammar.
    DASH_PARAMETERS_SELECTOR,
    QSELECT_SELECTOR,
    FILTER_SELECTOR,
    GETSEL_SELECTOR,
    SSX_SELECTOR,
    FIND_SELECTOR,
    FIND_REPORT_SELECTOR,
    // `AUDIT YES|NO|LOG` answers the `Fix any errors detected?` prompt (bare
    // AUDIT keeps the default No) and names the `AUDITCTL` log request.
    AUDIT_SELECTOR,
    // The 2026-11-07 utility batch argument forms.
    PURGE_SELECTOR,
    RENAME_SELECTOR,
    SETVAR_SELECTOR,
    SYSVARMONITOR_SELECTOR,
    // `SCALELISTEDIT` / `-SCALELISTEDIT`: the annotation-scale table's typed
    // writes (`ADD`/`RENAME`/`DELETE`) and the `?` report spelling.
    SCALELIST_SELECTOR,
    DASH_SCALELIST_SELECTOR,
    // `BASE <x> <y> [<z>]` and `LIMITS <min_x> <min_y> <max_x> <max_y>`: the two
    // vector-header writes. The bare selectors keep their read-only reports.
    super::header::BASE_SELECTOR,
    super::header::LIMITS_SELECTOR,
    MREDO_SELECTOR,
    MULTIPLE_SELECTOR,
    // The 2026-11-07 draw/modify batch argument forms.
    DONUT_SELECTOR,
    SOLID_SELECTOR,
    HELIX_SELECTOR,
    MLINE_SELECTOR,
    MLSTYLE_SELECTOR,
    TREX_SELECTOR,
    MPEDIT_SELECTOR,
    DRAWORDER_SELECTOR,
    // `ARRAYPATH <count> [ALIGN]`: the count/align ride the payload; the current
    // selection (path first) is the whole input.
    ARRAYPATH_SELECTOR,
    // The raster-image and table typed forms; grammars live in the sibling modules.
    IMAGEATTACH_SELECTOR,
    IMAGECLIP_SELECTOR,
    // `XCLIP RECT|POLY|ON|OFF|DELETE|INVERT|?`: block-local clip grammar in
    // `commands::xclip`; the command layer owns the semantic refusals.
    XCLIP_SELECTOR,
    // `CLIP RECT|POLY|ON|OFF|DELETE`: the generic boundary grammar over the two
    // clip models, in `commands::clip`. `CLIPIT` is deliberately absent: its
    // whole input is the selection, so a bare line reaches the command.
    CLIP_SELECTOR,
    IMAGEADJUST_SELECTOR,
    DASH_IMAGEADJUST_SELECTOR,
    TABLE_SELECTOR,
    DASH_TABLE_SELECTOR,
    TABLEDIT_SELECTOR,
    // `EXPORT <TOKEN>` (ADR-0052): the token only fixes the writer and the
    // picker's suggested extension for one interaction; the chosen target's
    // extension stays the authority (`export.format_mismatch` otherwise). A bare
    // `EXPORT` keeps the all-format picker.
    EXPORT_SELECTOR,
    // The post-creation table forms (2026-11-15): the cell-style copy, the style
    // field list and the CSV export destination. The bare `TABLESTYLE` keeps the
    // extended report, so only its argument form is listed here.
    MATCHCELL_SELECTOR,
    TABLESTYLE_SELECTOR,
    TABLEEXPORT_SELECTOR,
    VIEW_SELECTOR,
    VIEWGO_SELECTOR,
    VPOINT_SELECTOR,
    // `CVPORT <n>`: the session current-viewport marker of the active paper
    // layout (one-based position; the bare selector reports it).
    CVPORT_SELECTOR,
    // `ELEV <height>` and `FSMODE ON|OFF`: the two session tokens a create / `FS`
    // reads. Both bare selectors keep their read-only report.
    ELEV_SELECTOR,
    FSMODE_SELECTOR,
    // `MSTRETCH <base_x> <base_y> <delta_x> <delta_y> <x1> <y1> <x2> <y2> …`: the
    // typed line builds the very payload the STRETCH planner already consumed.
    MSTRETCH_SELECTOR,
    // `ALIGNSPACE <model_a> <paper_a> <model_b> <paper_b>`: the two alignment
    // point pairs the handler documents, as four 2D points.
    ALIGNSPACE_SELECTOR,
    // The 2026-11-07 layout/viewport batch argument forms, plus the typed
    // `PAGESETUP`/`PSETUPIN`/`MVIEW` writes. The dash spellings are distinct
    // extended selectors, not aliases, so each has its own row.
    LAYOUT_SELECTOR,
    LAYOUTMERGE_SELECTOR,
    LAYOUTWIZARD_SELECTOR,
    PAGESETUP_SELECTOR,
    DASH_PAGESETUP_SELECTOR,
    PSETUPIN_SELECTOR,
    DASH_PSETUPIN_SELECTOR,
    MVIEW_SELECTOR,
    VPORTS_SELECTOR,
    VPCLIP_SELECTOR,
    VPSCALE_SELECTOR,
    LAYVPI_SELECTOR,
    // `LAYFRZ VP [<index>,...]` (2026-11-24): the per-viewport freeze scope of
    // the layer-selection command. An argument form of `LAYFRZ`, not a new
    // command; the bare selector keeps the drawing-level freeze.
    LAYFRZ_SELECTOR,
    // ADR-0055 M3: the typed block-attribute command lines. The bare selectors
    // keep their report/no-op forms; these rows admit the write grammars.
    ATTDEF_SELECTOR,
    DASH_ATTDEF_SELECTOR,
    ATTEDIT_SELECTOR,
    DASH_ATTEDIT_SELECTOR,
    EATTEDIT_SELECTOR,
    ATTIPEDIT_SELECTOR,
    GATTE_SELECTOR,
    ATTSYNC_SELECTOR,
    ATTREDEF_SELECTOR,
    BATTORDER_SELECTOR,
    ATTDISP_SELECTOR,
    // `DIMSPACE` is a dimension argument form of its own (not the editing batch).
    DIMSPACE_SELECTOR,
    // The named style tables: the bare selector is the read-only report, and a
    // typed line is one post-import create/rename/delete/modify operation.
    STYLE_SELECTOR,
    DASH_STYLE_SELECTOR,
    MLEADERSTYLE_SELECTOR,
    // The 2026-11-07 dimension-editing batch argument forms.
    DIMSTYLE_SELECTOR,
    DIMTEDIT_SELECTOR,
    DIMEDIT_SELECTOR,
    DIMOVERRIDE_SELECTOR,
    DIMJOGLINE_SELECTOR,
    // `QDIM <mode>` is the argument form of the QDIM selector (the bare selector
    // keeps the Continuous default).
    QDIM_SELECTOR,
    // The 2026-11-07 annotation/group batch argument forms.
    GROUP_SELECTOR,
    UNGROUP_SELECTOR,
    GROUPEDIT_SELECTOR,
    QLEADER_SELECTOR,
    LEADER_SELECTOR,
    TEXTFIT_SELECTOR,
    HATCHSETORIGIN_SELECTOR,
    // `CENTERMARK [ARM <distance>|<factor>x]` and `CENTERLINE [EXTENSION
    // <distance>] [GAP <distance>]`: the typed options of the two associative
    // centre commands; the bare selectors keep their default geometry.
    crate::session::annot_arguments::CENTERMARK_SELECTOR,
    crate::session::annot_arguments::CENTERLINE_SELECTOR,
    // The 2026-11-07 extras batch typed forms. `SKETCH`/`MSTRETCH` have payload
    // paths only; `REGEN3` takes no argument.
    PAN_SELECTOR,
    TRACE_SELECTOR,
    MOCORO_SELECTOR,
    VIEWRES_SELECTOR,
    VSCURRENT_SELECTOR,
    // `IBL ON|OFF` (also `1`/`0`): the session PBR environment-lighting switch.
    // The bare selector toggles, so only the value form is listed here.
    IBL_SELECTOR,
    // `WIPEOUTFRAME <0|1|2>`: the system-variable spelling of `WIPEOUT Frames`
    // (same session token, one setter). The bare selector reports the value.
    WIPEOUTFRAME_SELECTOR,
    // `COLOR <aci>|BYLAYER|BYBLOCK` and its `-COLOR` prompt spelling: the session
    // current-colour token the create funnel stamps on new entities. The bare
    // selectors stay the read-only `color.report`.
    COLOR_SELECTOR,
    DASH_COLOR_SELECTOR,
    LINETYPE_SELECTOR,
    // The 2026-11 P3 option batch: OVERKILL's dialog set, REVCLOUD's mode/arc
    // length, WIPEOUT's Frames/Polyline pair, TEXTMASK's offset/mask type and
    // TXT2MTXT's settings. Each is an argument of its existing selector.
    payload_options::OVERKILL_SELECTOR,
    payload_options::REVCLOUD_SELECTOR,
    payload_options::WIPEOUT_SELECTOR,
    payload_options::TEXTMASK_SELECTOR,
    payload_options::TXT2MTXT_SELECTOR,
    // The 2026-11 paper-space/session/point-style typed forms; the grammars live
    // in `commands::paper_arguments`. A bare `VPLAYER`/`PTYPE`/`APERTURE`/
    // `REVCLOUDPROPERTIES` keeps its read-only report.
    VPLAYER_SELECTOR,
    MVSETUP_SELECTOR,
    APERTURE_SELECTOR,
    PTYPE_SELECTOR,
    REVCLOUDPROPERTIES_SELECTOR,
    SPACETRANS_SELECTOR,
    // The 2026-11-12 SHAPE family and its two typed siblings.
    SHAPE_SELECTOR,
    MINSERT_SELECTOR,
    ADDSELECTED_SELECTOR,
    // The 2026-11-12 MULTILEADER/MLEDIT/TOLERANCE typed forms. `MLEADERCOLLECT`
    // takes no argument, so it is deliberately absent.
    MLEDIT_SELECTOR,
    DASH_MLEDIT_SELECTOR,
    // `MLEADER [DOGLEG <length>] [DOGLEGANGLE <degrees>] [ATTACH <h> <v>] <four
    // coordinates> <text>`: the option prefix rides the ordinary create payload.
    MLEADER_SELECTOR,
    MLEADERALIGN_SELECTOR,
    MLEADEREDIT_SELECTOR,
    TOLERANCE_SELECTOR,
    // The 2026-11-12 in-place block-edit option lines: `REFSET ADD|REMOVE` and
    // `REFCLOSE SAVE|DISCARD`. A bare `REFSET` keeps the application's `ADD`
    // default; a bare `REFCLOSE` is `refclose.needs_option`.
    REFSET_SELECTOR,
    REFCLOSE_SELECTOR,
    // The session-only vocabulary controls (2026-11-12): `UNDEFINE <name>`,
    // `REDEFINE <name>` and `ALIASEDIT ADD|REMOVE|LIST`. They carry no
    // `CommandId`; `CadSession` resolves and applies them before the command
    // table. This row exists so the shell gate admits the typed line and queues
    // it verbatim.
    UNDEFINE_SELECTOR,
    REDEFINE_SELECTOR,
    ALIASEDIT_SELECTOR,
    // ADR-0056: the action recorder's seven typed command lines. The session
    // intercepts them before the generic argument dispatch (their grammar and
    // state live in `session::action_recorder`), but listing them here keeps the
    // chrome's `ARGUMENT_COMMANDS` mirror exact.
    super::super::action_recorder::ACTRECORD_SELECTOR,
    super::super::action_recorder::ACTSTOP_SELECTOR,
    super::super::action_recorder::DASH_ACTSTOP_SELECTOR,
    super::super::action_recorder::ACTUSERINPUT_SELECTOR,
    super::super::action_recorder::ACTUSERMESSAGE_SELECTOR,
    super::super::action_recorder::DASH_ACTUSERMESSAGE_SELECTOR,
    super::super::action_recorder::ACTMANAGER_SELECTOR,
];
