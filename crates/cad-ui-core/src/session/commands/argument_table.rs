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
/// batch takes its layer-state/laywalk/ltscale/layerpmode grammars and
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
    IMAGEADJUST_SELECTOR,
    DASH_IMAGEADJUST_SELECTOR,
    TABLE_SELECTOR,
    DASH_TABLE_SELECTOR,
    TABLEDIT_SELECTOR,
    VIEW_SELECTOR,
    VIEWGO_SELECTOR,
    VPOINT_SELECTOR,
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
    // `DIMSPACE` is a dimension argument form of its own (not the editing batch).
    DIMSPACE_SELECTOR,
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
    // The 2026-11-07 extras batch typed forms. `SKETCH`/`MSTRETCH` have payload
    // paths only; `REGEN3` takes no argument.
    PAN_SELECTOR,
    TRACE_SELECTOR,
    MOCORO_SELECTOR,
    VIEWRES_SELECTOR,
    VSCURRENT_SELECTOR,
    LINETYPE_SELECTOR,
    // The 2026-11 P3 option batch: OVERKILL's dialog set, REVCLOUD's mode/arc
    // length, WIPEOUT's Frames/Polyline pair, TEXTMASK's offset/mask type and
    // TXT2MTXT's settings. Each is an argument of its existing selector.
    payload_options::OVERKILL_SELECTOR,
    payload_options::REVCLOUD_SELECTOR,
    payload_options::WIPEOUT_SELECTOR,
    payload_options::TEXTMASK_SELECTOR,
    payload_options::TXT2MTXT_SELECTOR,
];
